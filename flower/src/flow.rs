pub mod command;
pub mod node;

use camino::Utf8PathBuf;
use futures::future::{BoxFuture, FutureExt, Shared};
use std::{
  collections::{BTreeMap, BTreeSet},
  future,
  sync::Arc,
};
use thiserror::Error;
use tokio::{sync::Mutex, task::JoinError};
use tracing::debug;

use crate::util::hash::PathHasher;
use command::InternalCommands;
use node::{Node, NodeError};

#[derive(Default)]
pub struct FlowDescription {
  pub dir: Utf8PathBuf,
  pub nodes: BTreeMap<String, Node>,
  pub internal_cmds: InternalCommands,
}
impl FlowDescription {
  pub fn node_dir(&self, name: &str) -> Utf8PathBuf {
    self.dir.join(name)
  }

  fn deps_from_paths<'a>(&self, paths: impl Iterator<Item = &'a Utf8PathBuf>) -> BTreeSet<String> {
    paths
      .filter_map(|path| path.strip_prefix(&self.dir).ok())
      .filter_map(|path| path.iter().next())
      .map(str::to_string)
      .collect()
  }
}

type NodeId = String;
pub type FlowTask = Shared<BoxFuture<'static, Result<(), FlowError>>>;

#[derive(Default)]
pub struct FlowRunner {
  flow: FlowDescription,
  spawned: BTreeMap<NodeId, FlowTask>,
  spawning: BTreeSet<NodeId>,
  hasher: Arc<Mutex<PathHasher>>,
}
impl From<FlowDescription> for FlowRunner {
  fn from(flow: FlowDescription) -> Self {
    Self { flow, ..Default::default() }
  }
}

impl FlowRunner {
  /// Recursively spawns the node and its dependencies, returning a shared future.
  pub fn spawn_node(&mut self, name: &str, force_rerun: bool) -> FlowTask {
    if let Some(task) = self.spawned.get(name) {
      return task.clone();
    }

    let flow_err = |err| future::ready(Err(err)).boxed().shared();

    let Some(node) = self.flow.nodes.get(name).cloned() else {
      return flow_err(FlowError::NodeNotFound(name.into()));
    };

    if !self.spawning.insert(name.into()) {
      return flow_err(FlowError::CircularDependency(name.into()));
    }

    let dep_names = self.flow.deps_from_paths(node.required_paths.iter());
    if dep_names.contains(name) {
      return flow_err(FlowError::RequiringOwnPath(name.into()));
    }
    if let Some(path) = node.undeclared_arg_files().next() {
      return flow_err(FlowError::UndeclaredArgFile(name.into(), path.clone()));
    }
    debug!("{name} depends on {dep_names:?}.");

    let dep_tasks: Vec<_> = dep_names.iter().map(|dep| self.spawn_node(dep, force_rerun)).collect();

    let node_fut = {
      let node_dir = self.flow.node_dir(name);
      let name = name.to_string();
      let hasher = self.hasher.clone();
      let internal_cmds = self.flow.internal_cmds.clone();

      async move {
        for dep_task in dep_tasks {
          dep_task.await?;
        }
        node
          .run(&node_dir, &internal_cmds, hasher, force_rerun)
          .await
          .map_err(|err| FlowError::NodeError(name, err.into()))?;
        Ok(())
      }
    };

    let shared_task = tokio::spawn(node_fut)
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
}

#[derive(Clone, Debug, Error)]
pub enum FlowError {
  #[error("Async task didn't complete: {0}")]
  PanicOrAbort(Arc<JoinError>),
  #[error("Node '{0}' could not be found.")]
  NodeNotFound(String),
  #[error("Circular dependency involving node '{0}'.")]
  CircularDependency(String),
  #[error("Node '{0}' requires it's own path.")]
  RequiringOwnPath(String),
  #[error("Node '{0}' reads arguments from {1}, which isn't in its directory or required paths.")]
  UndeclaredArgFile(String, Utf8PathBuf),
  #[error("Node '{0}' failure: {1}")]
  NodeError(String, Arc<NodeError>),
}
impl From<JoinError> for FlowError {
  fn from(error: JoinError) -> Self {
    Self::PanicOrAbort(Arc::new(error))
  }
}
