use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs;
use tokio::io;
use tokio::sync::Mutex;
use tracing::{debug, info};

use super::command::CommandError;
use super::command::{Command, CommandResult, ExitStatus};
use crate::clean_dir;
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
  /// Files commands read must be inside the node's directory
  /// (a relative path without `..`) or be covered by a required path,
  /// otherwise changes to them wouldn't cause a rerun.
  pub fn undeclared_input_files(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.commands.iter().flat_map(Command::input_files).filter(|path| {
      let in_node_dir = path
        .components()
        .all(|component| matches!(component, Utf8Component::Normal(_) | Utf8Component::CurDir));
      !in_node_dir && !self.required_paths.iter().any(|required| path.starts_with(required))
    })
  }

  pub async fn rerun_required(
    &self,
    node_dir: &Utf8Path,
    hasher: &mut PathHasher,
  ) -> NodeResult<bool> {
    let name = || node_dir.file_name().unwrap();

    let required_paths = self.required_paths.iter();

    if !Self::manifest_exists(node_dir).await? || !NodeRunInfo::manifest_exists(node_dir).await? {
      debug!("{} requires rerun due to missing manifest(s).", name());
      return Ok(true);
    }

    if Self::read(node_dir).await? != *self {
      debug!("{} requires rerun due to changed info manifest.", name());
      return Ok(true);
    }

    let info = NodeRunInfo::read(node_dir).await?;

    if self
      .commands
      .iter()
      .zip(info.results.iter())
      .any(|(cmd, result)| !cmd.no_check() && result.status != 0)
    {
      debug!("{} requires rerun due to failure last run.", name());
      return Ok(true);
    }

    let hashes_match = info.hashes_match(required_paths, hasher).await?;
    Ok(!hashes_match)
  }

  pub async fn run(
    &self,
    node_dir: &Utf8Path,
    hasher: Arc<Mutex<PathHasher>>,
    force_rerun: bool,
  ) -> NodeResult<()> {
    let name = || node_dir.file_name().unwrap();

    {
      let mut hasher = hasher.lock().await;
      if !force_rerun && !self.rerun_required(node_dir, &mut hasher).await? {
        info!("Skipping {}.", name());
        return Ok(());
      }
      // We may change these paths, so we should clear the cache.
      hasher.remove(node_dir);
    }

    info!("Running {}.", name());

    let mut info = NodeRunInfo::default();

    clean_dir(node_dir).await.map_err(NodeError::CreateNodeDir)?;
    self.write(node_dir).await?;

    for command in &self.commands {
      let exec_result = command.run(node_dir).await.map_err(NodeError::Execute)?;
      let status = exec_result.status;

      info.results.push(exec_result);

      if !command.no_check() && status != 0 {
        if let Ok(hashes) = self.required_path_hashes(hasher).await {
          info.required_path_hashes = hashes;
        }
        info.write(node_dir).await?;
        let run_info = NodeRunInfo::manifest_path(node_dir);
        return Err(NodeError::NonZeroExitStatus { status, run_info });
      }
    }

    info.required_path_hashes = self.required_path_hashes(hasher).await?;

    info.write(node_dir).await?;
    Ok(())
  }

  async fn required_path_hashes(
    &self,
    hasher: Arc<Mutex<PathHasher>>,
  ) -> NodeResult<BTreeMap<Utf8PathBuf, PathHash>> {
    let mut hasher = hasher.lock().await;
    let mut hashes = BTreeMap::<Utf8PathBuf, PathHash>::new();
    for path in &self.required_paths {
      let hash = hasher.hash(path).await?;
      hashes.insert(path.into(), hash);
    }
    Ok(hashes)
  }
}

#[derive(Default, Serialize, Deserialize)]
pub struct NodeRunInfo {
  results: Vec<CommandResult>,
  required_path_hashes: BTreeMap<Utf8PathBuf, PathHash>,
}

impl NodeRunInfo {
  pub async fn hashes_match<'a>(
    &self,
    paths: impl Iterator<Item = &'a Utf8PathBuf>,
    hasher: &mut PathHasher,
  ) -> Result<bool, HashingError> {
    for path in paths {
      let Some(previous_hash) = self.required_path_hashes.get(path) else { return Ok(false) };
      let hash = hasher.hash(path).await?;
      if hash != *previous_hash {
        return Ok(false);
      }
    }
    Ok(true)
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
  #[error("Failed to create node directory: {0}")]
  CreateNodeDir(io::Error),
  #[error("Failed to run command: {0}")]
  Execute(#[from] CommandError),
  #[error("Executable terminated with status {status} (see {run_info})")]
  NonZeroExitStatus { status: ExitStatus, run_info: Utf8PathBuf },
}
