use futures::future::{BoxFuture, FutureExt, Shared};
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, HashSet},
  future,
  path::PathBuf,
  sync::Arc,
};
use tokio::task::JoinError;

use crate::node::{Node, NodeRunError};

#[derive(Serialize, Deserialize)]
pub struct FlowDescription {
  pub dir: PathBuf,
  pub nodes: BTreeMap<String, Node>,
}

pub type FlowTask = Shared<BoxFuture<'static, Result<(), FlowRunError>>>;

pub struct FlowRunner {
  flow: FlowDescription,
  spawned: BTreeMap<String, FlowTask>,
  spawning: HashSet<String>,
}
impl From<FlowDescription> for FlowRunner {
  fn from(flow: FlowDescription) -> Self {
    Self { flow, spawned: BTreeMap::new(), spawning: HashSet::new() }
  }
}

impl FlowRunner {
  /// Recursively spawns the node and its dependencies, returning a shared future.
  pub fn spawn_node(&mut self, name: &str) -> FlowTask {
    if let Some(task) = self.spawned.get(name) {
      return task.clone();
    }

    let flow_err = |err| future::ready(Err(err)).boxed().shared();

    let Some(node) = self.flow.nodes.get(name).cloned() else {
      return flow_err(FlowRunError::NodeNotFound(name.into()));
    };

    if !self.spawning.insert(name.into()) {
      return flow_err(FlowRunError::CircularDependency(name.into()));
    }

    let dep_names = self.get_dependencies(node.required_paths.iter());
    if dep_names.contains(name) {
      return flow_err(FlowRunError::RequiringOwnPath(name.into()));
    }

    let dep_tasks: Vec<_> = dep_names.iter().map(|dep| self.spawn_node(dep)).collect();

    let node_fut = {
      let node_dir = self.flow.dir.join(name);
      let name = name.to_string();

      async move {
        for dep_task in dep_tasks {
          dep_task.await?;
        }
        println!("Executing node: {}", &name);
        node.run(&node_dir).await.map_err(|err| FlowRunError::NodeError(name, err))?;
        Ok(())
      }
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

    let name = self.spawning.take(name).expect("The name is expected to be in spawning.");
    self.spawned.insert(name, shared_task.clone());

    shared_task
  }

  fn get_dependencies<'a>(&self, paths: impl Iterator<Item = &'a PathBuf>) -> HashSet<String> {
    paths
      .filter_map(|path| path.strip_prefix(&self.flow.dir).ok())
      .filter_map(|path| path.iter().next())
      .map(|name| name.to_str().unwrap().to_string())
      .collect()
  }
}

#[derive(Clone, Debug)]
pub enum FlowRunError {
  PanicOrAbort(Arc<JoinError>),
  NodeNotFound(String),
  CircularDependency(String),
  RequiringOwnPath(String),
  NodeError(String, NodeRunError),
}
impl From<JoinError> for FlowRunError {
  fn from(error: JoinError) -> Self {
    Self::PanicOrAbort(Arc::new(error))
  }
}
