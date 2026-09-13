use futures::future::{BoxFuture, FutureExt, Shared};
use rmp_serde::{Serializer, config::BytesMode, encode};
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, HashSet}, ffi::OsString, future, path::{Path, PathBuf}, sync::Arc
};
use tokio::io::{self, AsyncWriteExt};
use tokio::{fs, process::Command, task::JoinError};

pub type NodeId = usize;
pub type ExecIdx = u32;
pub type ExitCode = i32;

#[derive(Serialize, Deserialize)]
pub struct FlowDescription {
  pub dir: PathBuf,
  pub nodes: Vec<Node>,
}

type SharedTask = Shared<BoxFuture<'static, Result<(), FlowRunError>>>;

pub struct FlowRunner {
  pub flow: FlowDescription,
  spawned: BTreeMap<NodeId, SharedTask>,
  spawning: HashSet<NodeId>,
}
impl From<FlowDescription> for FlowRunner {
  fn from(flow: FlowDescription) -> Self {
    Self { flow, spawned: BTreeMap::new(), spawning: HashSet::new() }
  }
}

impl FlowRunner {
  /// Recursively spawns the node and its dependencies, returning a shared future.
  pub fn spawn_node(&mut self, id: NodeId) -> SharedTask {
    if let Some(task) = self.spawned.get(&id) {
      return task.clone();
    }

    let flow_err = |err| future::ready(Err(err)).boxed().shared();

    let Some(node) = self.flow.nodes.get(id).cloned() else {
      return flow_err(FlowRunError::NodeIdNotFound(id));
    };
    let node_dir = self.flow.dir.join(&node.name);

    if !self.spawning.insert(id) {
      return flow_err(FlowRunError::CircularDependency(node.name));
    }

    let dep_tasks: Vec<_> = node.dependencies.iter().map(|dep| self.spawn_node(*dep)).collect();

    let node_fut = async move {
      let to_flow_err = |err| FlowRunError::NodeError(id, err);

      for dep_task in dep_tasks {
        dep_task.await?;
      }

      println!("Executing node: {}", node.name);
      node.run(&node_dir).await.map_err(to_flow_err)?;
      Ok(())
    };

    let handle = tokio::spawn(node_fut);
    let shared_task = handle
      .map(|res| match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(node_run_error)) => Err(node_run_error),
        Err(join_error) => Err(join_error.into()),
      })
      .boxed()
      .shared();

    self.spawned.insert(id, shared_task.clone());
    self.spawning.remove(&id);

    shared_task
  }
}

#[derive(Clone, Debug)]
pub enum FlowRunError {
  PanicOrAbort(Arc<JoinError>),
  NodeIdNotFound(NodeId),
  CircularDependency(String),
  NodeError(NodeId, NodeRunError),
}
impl From<JoinError> for FlowRunError {
  fn from(error: JoinError) -> Self {
    Self::PanicOrAbort(Arc::new(error))
  }
}

#[derive(Clone, Debug)]
pub enum NodeRunError {
  Serdes(Arc<encode::Error>),
  Io(Arc<io::Error>),
  NonZeroExitCode(ExitCode),
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

#[derive(Clone, Serialize, Deserialize)]
pub struct Node {
  pub name: String,
  pub dependencies: Vec<NodeId>,
  pub execs: Vec<ExecRunInfo>,
}

impl Node {
  async fn run(&self, node_dir: &Path) -> Result<(), NodeRunError> {
    let mut info = NodeRunInfo::default();

    fs::create_dir_all(node_dir).await?;

    for exec in &self.execs {
      let exec_result = exec.run(node_dir).await?;
      let status = exec_result.status;

      info.exec_results.push(exec_result);

      if !exec.no_check && status != 0 {
        info.write(node_dir).await?;
        return Err(NodeRunError::NonZeroExitCode(status));
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
  const INFO_FILE_NAME: &'static str = "info.msgpack";

  async fn write(&self, node_dir: &Path) -> Result<(), NodeRunError> {
    let mut info_serialized = Vec::new();
    let mut serializer =
      Serializer::new(&mut info_serialized).with_struct_map().with_bytes(BytesMode::ForceAll);
    self.serialize(&mut serializer)?;

    let info_path = node_dir.join(Self::INFO_FILE_NAME);
    fs::File::create(info_path).await?.write_all(&info_serialized).await?;
    Ok(())
  }
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ExecRunInfo {
  pub env: BTreeMap<OsString, OsString>,
  pub exec: PathBuf,
  pub no_check: bool,
  pub args: Vec<OsString>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecRunResult {
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
  pub status: i32,
}

impl ExecRunInfo {
  pub async fn run(&self, run_dir: &Path) -> io::Result<ExecRunResult> {
    let output = Command::from(self).current_dir(run_dir).output().await?;
    Ok(ExecRunResult {
      stdout: output.stdout,
      stderr: output.stderr,
      status: output.status.code().unwrap_or(0),
    })
  }
}

impl From<&ExecRunInfo> for Command {
  fn from(info: &ExecRunInfo) -> Self {
    let mut cmd = Self::new(&info.exec);
    cmd.env_clear().envs(&info.env).args(&info.args);
    cmd
  }
}
