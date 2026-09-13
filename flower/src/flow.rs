use futures::future::{BoxFuture, FutureExt, Shared};
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, HashSet},
  future,
  path::PathBuf,
  sync::Arc,
};
use tokio::task::JoinError;

use crate::node::{Node, NodeId, NodeRunError};

#[derive(Serialize, Deserialize)]
pub struct FlowDescription {
  pub dir: PathBuf,
  pub nodes: Vec<Node>,
}

pub type FlowTask = Shared<BoxFuture<'static, Result<(), FlowRunError>>>;

pub struct FlowRunner {
  pub flow: FlowDescription,
  spawned: BTreeMap<NodeId, FlowTask>,
  spawning: HashSet<NodeId>,
}
impl From<FlowDescription> for FlowRunner {
  fn from(flow: FlowDescription) -> Self {
    Self { flow, spawned: BTreeMap::new(), spawning: HashSet::new() }
  }
}

impl FlowRunner {
  /// Recursively spawns the node and its dependencies, returning a shared future.
  pub fn spawn_node(&mut self, id: NodeId) -> FlowTask {
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
