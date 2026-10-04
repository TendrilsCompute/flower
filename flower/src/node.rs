use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs;
use tokio::io;
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::clean_dir;
use crate::exec::{ExecRunInfo, ExecRunResult, ExitStatus};
use crate::hash::{HashingError, PathHash, PathHasher};
use crate::serialization::{self, SerializationError};

type NodeResult<T> = Result<T, NodeError>;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
  pub required_paths: Vec<Utf8PathBuf>,
  pub execs: Vec<ExecRunInfo>,
}

impl Node {
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
      .execs
      .iter()
      .zip(info.exec_results.iter())
      .any(|(exec, result)| !exec.no_check && result.status != 0)
    {
      debug!("{} requires rerun due to failure last run.", name());
      return Ok(true);
    }

    let hashes_match = info.hashes_match(required_paths, hasher).await?;
    Ok(!hashes_match)
  }

  pub async fn run(&self, node_dir: &Utf8Path, hasher: Arc<Mutex<PathHasher>>) -> NodeResult<()> {
    let name = || node_dir.file_name().unwrap();

    {
      let mut hasher = hasher.lock().await;
      if !self.rerun_required(node_dir, &mut hasher).await? {
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

    for exec in &self.execs {
      let exec_result = exec.run(node_dir).await.map_err(NodeError::Execute)?;
      let status = exec_result.status;

      info.exec_results.push(exec_result);

      if !exec.no_check && status != 0 {
        if let Ok(hashes) = self.required_path_hashes(hasher).await {
          info.required_path_hashes = hashes;
        }
        info.write(node_dir).await?;
        return Err(NodeError::NonZeroExitStatus(status));
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
      let hash = hasher.hash(path)?;
      hashes.insert(path.into(), hash);
    }
    Ok(hashes)
  }
}

#[derive(Default, Serialize, Deserialize)]
pub struct NodeRunInfo {
  exec_results: Vec<ExecRunResult>,
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
      let hash = hasher.hash(path)?;
      if hash != *previous_hash {
        return Ok(false);
      }
    }
    Ok(true)
  }
}

impl NodeManifest for Node {
  const MANIFEST_FILE: &'static str = "info.msgpack";
}

impl NodeManifest for NodeRunInfo {
  const MANIFEST_FILE: &'static str = "run.msgpack";
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
  Serialization(SerializationError),
  #[error("Hashing error: {0}")]
  Hashing(HashingError),
  #[error("Failed to create node directory: {0}")]
  CreateNodeDir(io::Error),
  #[error("Failed to run executable: {0}")]
  Execute(io::Error),
  #[error("Executable terminiated with status {0}")]
  NonZeroExitStatus(ExitStatus),
}
impl From<SerializationError> for NodeError {
  fn from(error: SerializationError) -> Self {
    Self::Serialization(error)
  }
}
impl From<HashingError> for NodeError {
  fn from(error: HashingError) -> Self {
    Self::Hashing(error)
  }
}
