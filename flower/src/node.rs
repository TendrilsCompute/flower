use rmp_serde::{Serializer, config::BytesMode, encode};
use serde::{Deserialize, Serialize};
use std::{
  path::Path, sync::Arc
};
use tokio::io::{self, AsyncWriteExt};
use tokio::fs;

use crate::exec::{ExecRunInfo, ExecRunResult, ExitStatus};

pub type NodeId = usize;

#[derive(Clone, Serialize, Deserialize)]
pub struct Node {
  pub name: String,
  pub dependencies: Vec<NodeId>,
  pub execs: Vec<ExecRunInfo>,
}

impl Node {
  pub async fn run(&self, node_dir: &Path) -> Result<(), NodeRunError> {
    let mut info = NodeRunInfo::default();

    fs::create_dir_all(node_dir).await?;

    for exec in &self.execs {
      let exec_result = exec.run(node_dir).await?;
      let status = exec_result.status;

      info.exec_results.push(exec_result);

      if !exec.no_check && status != 0 {
        info.write(node_dir).await?;
        return Err(NodeRunError::NonZeroExitStatus(status));
      }
    }

    info.write(node_dir).await?;
    Ok(())
  }
}

#[derive(Default, Serialize, Deserialize)]
pub struct NodeRunInfo {
  exec_results: Vec<ExecRunResult>,
}

impl NodeRunInfo {
  const INFO_FILE: &'static str = "info.msgpack";

  async fn write(&self, node_dir: &Path) -> Result<(), NodeRunError> {
    let mut info_serialized = Vec::new();
    let mut serializer =
      Serializer::new(&mut info_serialized).with_struct_map().with_bytes(BytesMode::ForceAll);
    self.serialize(&mut serializer)?;

    let info_path = node_dir.join(Self::INFO_FILE);
    fs::File::create(info_path).await?.write_all(&info_serialized).await?;
    Ok(())
  }
}

#[derive(Clone, Debug)]
pub enum NodeRunError {
  Serdes(Arc<encode::Error>),
  Io(Arc<io::Error>),
  NonZeroExitStatus(ExitStatus),
}
impl From<io::Error> for NodeRunError {
  fn from(error: io::Error) -> Self {
    Self::Io(Arc::new(error))
  }
}
impl From<encode::Error> for NodeRunError {
  fn from(error: encode::Error) -> Self {
    Self::Serdes(Arc::new(error))
  }
}

