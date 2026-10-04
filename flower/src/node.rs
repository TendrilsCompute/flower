use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::fs;
use tokio::io;
use tokio::sync::Mutex;
use tracing::info;

use crate::exec::{ExecRunInfo, ExecRunResult, ExitStatus};
use crate::hash::{HashingError, PathHash, PathHasher};
use crate::serialization::{self, SerializationError};

type NodeResult<T> = Result<T, NodeError>;

#[derive(Clone, Serialize, Deserialize)]
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
    let required_paths = self.required_paths.iter();

    if !NodeRunInfo::file_exists(node_dir).await? {
      return Ok(true);
    }

    let info = NodeRunInfo::read(node_dir).await?;
    let hashes_match = info.hashes_match(required_paths, hasher).await?;
    Ok(!hashes_match)
  }

  pub async fn run(
    &self,
    node_dir: &Utf8Path,
    hasher: Arc<Mutex<PathHasher>>,
  ) -> Result<(), NodeError> {
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

    fs::create_dir_all(node_dir).await.map_err(NodeError::CreateNodeDir)?;

    for exec in &self.execs {
      let exec_result = exec.run(node_dir).await.map_err(NodeError::Execute)?;
      let status = exec_result.status;

      info.exec_results.push(exec_result);

      if !exec.no_check && status != 0 {
        info.write(node_dir).await?;
        return Err(NodeError::NonZeroExitStatus(status));
      }
    }

    {
      let mut hasher = hasher.lock().await;
      for path in &self.required_paths {
        let hash = hasher.hash(path)?;
        info.required_path_hashes.insert(path.into(), hash);
      }
    }

    info.write(node_dir).await?;
    Ok(())
  }
}

#[derive(Default, Serialize, Deserialize)]
pub struct NodeRunInfo {
  exec_results: Vec<ExecRunResult>,
  required_path_hashes: BTreeMap<Utf8PathBuf, PathHash>,
}

impl NodeRunInfo {
  const INFO_FILE: &'static str = "info.msgpack";

  fn info_path(node_dir: &Utf8Path) -> Utf8PathBuf {
    node_dir.join(NodeRunInfo::INFO_FILE)
  }

  pub async fn file_exists(node_dir: &Utf8Path) -> Result<bool, SerializationError> {
    let path = Self::info_path(node_dir);
    fs::try_exists(&path).await.map_err(serialization::io_err(&path))
  }

  pub async fn read(node_dir: &Utf8Path) -> Result<Self, SerializationError> {
    serialization::read(&Self::info_path(node_dir)).await
  }

  async fn write(&self, node_dir: &Utf8Path) -> Result<(), SerializationError> {
    serialization::write(self, &Self::info_path(node_dir)).await
  }

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

#[derive(Debug)]
pub enum NodeError {
  Serialization(SerializationError),
  Hashing(HashingError),
  CreateNodeDir(io::Error),
  Execute(io::Error),
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
