use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use tokio::fs;
use tracing::debug;

use super::command::CommandError;
use super::command::{Command, CommandResult, ExitStatus};
use super::path::{InvalidPath, NodePath};
use crate::util::{
  hash::{HashingError, PathHash, PathHasher},
  serialization::{self, SerializationError},
};

type NodeResult<T> = Result<T, NodeError>;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
  pub required_paths: Vec<Utf8PathBuf>,
  pub commands: Vec<Command>,
}

impl Node {
  /// The nodes this node depends on, i.e. those its required paths are in.
  /// It's an error for a required path to be in the node's own directory.
  pub fn deps(&self) -> NodeResult<BTreeSet<String>> {
    self
      .required_paths
      .iter()
      .filter_map(|path| {
        Some(match NodePath::parse(path) {
          Ok(NodePath::External(_)) => return None,
          Ok(NodePath::OtherNode { node, .. }) => Ok(node.to_string()),
          Ok(NodePath::Local(_)) => Err(NodeError::RequiringOwnPath),
          Err(err) => Err(NodeError::InvalidPath(err)),
        })
      })
      .collect()
  }

  /// Checks the node's paths are valid and that it declares what it uses from outside its
  /// directory, for a flow in `flow_dir`. Invalid required paths are reported by [`Self::deps`].
  pub fn check_paths(&self, flow_dir: &Utf8Path) -> NodeResult<()> {
    let inputs = self.commands.iter().flat_map(Command::input_files);
    let execs = self.commands.iter().filter_map(Command::exec);
    for input_path in inputs.clone() {
      if let NodePath::Local(_) = NodePath::parse(input_path)? {
        continue;
      }
      if !self.is_required(input_path) {
        // Non local inputs should be in the required paths.
        return Err(NodeError::UndeclaredInputFile(input_path.clone()));
      }
    }
    for output_path in self.commands.iter().flat_map(Command::output_files) {
      // All output paths should be local.
      if let NodePath::Local(_) = NodePath::parse(output_path)? {
        continue;
      }
      return Err(NodeError::NonLocalOutputFile(output_path.clone()));
    }
    for exec_path in execs.clone() {
      // Executables from other nodes should be in the required paths.
      if let NodePath::OtherNode { .. } = NodePath::parse(exec_path)?
        && !self.is_required(exec_path)
      {
        return Err(NodeError::UndeclaredExecutable(exec_path.to_owned()));
      }
    }
    for path in self.required_paths.iter().chain(inputs).chain(execs) {
      if let Ok(NodePath::External(path)) = NodePath::parse(path)
        && (path.starts_with(flow_dir) || flow_dir.starts_with(path))
      {
        return Err(NodeError::ExternalPathOverlapsFlow(path.to_owned()));
      }
    }
    Ok(())
  }

  /// Whether `path` is covered by a required path.
  fn is_required(&self, path: &Utf8Path) -> bool {
    self.required_paths.iter().any(|required| path.starts_with(required))
  }

  /// Whether the node needs to run in `node_dir`, rather than reuse its `previous` run.
  /// `node_dir` may not exist yet, but its siblings must be this run's dependencies.
  pub async fn rerun_required(
    &self,
    previous: &Utf8Path,
    node_dir: &Utf8Path,
    hasher: &PathHasher,
  ) -> NodeResult<bool> {
    let name = || node_dir.file_name().unwrap();

    if !Self::manifest_exists(previous).await? || !NodeRunInfo::manifest_exists(previous).await? {
      debug!("{} requires rerun due to missing manifest(s).", name());
      return Ok(true);
    }

    if Self::read(previous).await? != *self {
      debug!("{} requires rerun due to changed info manifest.", name());
      return Ok(true);
    }

    let info = NodeRunInfo::read(previous).await?;

    if self
      .commands
      .iter()
      .zip(info.commands.iter())
      .any(|(cmd, result)| !cmd.no_check() && result.status != 0)
    {
      debug!("{} requires rerun due to failure last run.", name());
      return Ok(true);
    }

    let hashes = self.required_path_hashes(node_dir, hasher).await?;
    Ok(hashes != info.required_paths)
  }

  /// Runs the commands in `node_dir`, an existing empty directory
  /// whose siblings are the directories of the node's dependencies.
  pub async fn run(&self, node_dir: &Utf8Path, hasher: &PathHasher) -> NodeResult<()> {
    let required_paths = self.required_path_hashes(node_dir, hasher).await?;
    let mut info = NodeRunInfo { required_paths, ..Default::default() };

    self.write(node_dir).await?;

    for command in &self.commands {
      let exec_result = command.run(node_dir).await.map_err(NodeError::Execute)?;
      let status = exec_result.status;

      info.commands.push(exec_result);

      if !command.no_check() && status != 0 {
        info.write(node_dir).await?;
        let run_info = NodeRunInfo::manifest_path(node_dir);
        return Err(NodeError::NonZeroExitStatus { status, run_info });
      }
    }

    info.write(node_dir).await?;
    Ok(())
  }

  /// Keyed by the paths as the node defines them, but hashed where they are for `node_dir`.
  async fn required_path_hashes(
    &self,
    node_dir: &Utf8Path,
    hasher: &PathHasher,
  ) -> NodeResult<BTreeMap<Utf8PathBuf, PathHash>> {
    let mut hashes = BTreeMap::new();
    for path in &self.required_paths {
      let hash = hasher.hash(&NodePath::parse(path)?.resolve(node_dir)).await?;
      hashes.insert(path.clone(), hash);
    }
    Ok(hashes)
  }
}

#[derive(Default, Serialize, Deserialize)]
pub struct NodeRunInfo {
  commands: Vec<CommandResult>,
  required_paths: BTreeMap<Utf8PathBuf, PathHash>,
}

impl NodeRunInfo {
  /// Written when the node finishes, so requiring it orders after the node.
  pub fn manifest_path(node_dir: &Utf8Path) -> Utf8PathBuf {
    <Self as NodeManifest>::manifest_path(node_dir)
  }

  /// The stderr of the last command run, or its stdout if it has no stderr.
  pub fn last_output(&self) -> &[u8] {
    let Some(result) = self.commands.last() else { return &[] };
    if result.stderr.is_empty() { &result.stdout } else { &result.stderr }
  }
}

impl NodeManifest for Node {
  const MANIFEST_FILE: &'static str = "info.msgpackz";
}

impl NodeManifest for NodeRunInfo {
  const MANIFEST_FILE: &'static str = "run.msgpackz";
}

trait NodeManifest
where
  for<'de> Self: Deserialize<'de> + Serialize,
{
  const MANIFEST_FILE: &'static str;

  fn manifest_path(node_dir: &Utf8Path) -> Utf8PathBuf {
    node_dir.join(Self::MANIFEST_FILE)
  }

  async fn manifest_exists(node_dir: &Utf8Path) -> Result<bool, SerializationError> {
    let path = Self::manifest_path(node_dir);
    fs::try_exists(&path).await.map_err(serialization::io_err(&path))
  }

  async fn read(node_dir: &Utf8Path) -> Result<Self, SerializationError> {
    serialization::read(&Self::manifest_path(node_dir)).await
  }

  async fn write(&self, node_dir: &Utf8Path) -> Result<(), SerializationError> {
    serialization::write(self, &Self::manifest_path(node_dir)).await
  }
}

#[derive(Debug, Error)]
pub enum NodeError {
  #[error("Serialization: {0}")]
  Serialization(#[from] SerializationError),
  #[error("Hashing error: {0}")]
  Hashing(#[from] HashingError),
  #[error("Node requires its own path.")]
  RequiringOwnPath,
  #[error("Node has an undeclared input file {0}.")]
  UndeclaredInputFile(Utf8PathBuf),
  #[error("Node runs {0}, which isn't covered by its required paths.")]
  UndeclaredExecutable(Utf8PathBuf),
  #[error("Node uses {0}, which overlaps the flow directory.")]
  ExternalPathOverlapsFlow(Utf8PathBuf),
  #[error("Node writes to a non-local file {0}.")]
  NonLocalOutputFile(Utf8PathBuf),
  #[error(transparent)]
  InvalidPath(#[from] InvalidPath),
  #[error("Failed to run command: {0}")]
  Execute(#[from] CommandError),
  #[error("Executable terminated with status {status} (see {run_info})")]
  NonZeroExitStatus { status: ExitStatus, run_info: Utf8PathBuf },
}
