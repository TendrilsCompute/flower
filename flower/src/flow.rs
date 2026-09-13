use camino::Utf8PathBuf;
use futures::future::{BoxFuture, FutureExt, Shared};
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, BTreeSet},
  sync::Arc,
};
use tokio::{sync::RwLock, task::JoinError};
use tracing::debug;

use crate::{
  hash::PathHasher,
  node::{Node, NodeError},
};

#[derive(Default, Serialize, Deserialize)]
pub struct FlowDescription {
  pub dir: Utf8PathBuf,
  pub nodes: BTreeMap<String, Node>,
}
impl FlowDescription {
  fn deps_from_paths<'a>(&self, paths: impl Iterator<Item = &'a Utf8PathBuf>) -> BTreeSet<String> {
    paths
      .filter_map(|path| path.strip_prefix(&self.dir).ok())
      .filter_map(|path| path.iter().next())
      .map(str::to_string)
      .collect()
  }
}

type NodeName = String;
pub type FlowTask = Shared<BoxFuture<'static, Result<(), FlowError>>>;

#[derive(Default)]
struct FlowRunState {
  flow: FlowDescription,
  hasher: PathHasher,
  tasks: BTreeMap<NodeName, FlowTask>,
}

#[derive(Default)]
pub struct FlowRunner {
  state: Arc<RwLock<FlowRunState>>,
}
impl From<FlowDescription> for FlowRunner {
  fn from(flow: FlowDescription) -> Self {
    let state = FlowRunState { flow, ..Default::default() };
    FlowRunner { state: Arc::new(RwLock::new(state)) }
  }
}

impl FlowRunner {
  pub fn spawn_node(&self, name: impl Into<String>) -> FlowTask {
    Self::spawn_dep(self.state.clone(), name.into(), vec![])
  }

  fn spawn_dep(
    state: Arc<RwLock<FlowRunState>>,
    name: String,
    mut path: Vec<String>,
  ) -> FlowTask {
    let node_fut = async move {
      if path.contains(&name) {
        path.push(name);
        return Err(FlowError::CircularDependency(path));
      }

      let (node, node_dir) = {
        let state = state.read().await;
        if let Some(task) = state.tasks.get(&name).cloned() {
          return task.await;
        }

        let node_dir = state.flow.dir.join(&name);
        let Some(node) = state.flow.nodes.get(&name).cloned() else {
          return Err(FlowError::NodeNotFound(name));
        };
        (node, node_dir)
      };

      let dep_names = {
        let mut state = state.write().await;

        let rerun_paths = match node.rerun_required(&node_dir, &mut state.hasher).await {
          Err(err) => return Err(FlowError::NodeError(name, Arc::new(err))),
          Ok(None) => {
            debug!("Skipping '{name}'. No change since last run.");
            return Ok(());
          }
          Ok(Some(rerun_paths)) => rerun_paths,
        };

        state.flow.deps_from_paths(rerun_paths.into_iter())
      };

      debug!("'{name}' requires {dep_names:?} to be run.");

      if dep_names.contains(&name) {
        return Err(FlowError::RequiringOwnPath(name));
      }

      path.push(name);

      let mut dep_tasks = vec![];
      for dep in dep_names.into_iter() {
        dep_tasks.push(Self::spawn_dep(state.clone(), dep, path.clone()));
      }

      for dep_task in dep_tasks {
        dep_task.await?;
      }

      debug!("Running '{}'", path.last().unwrap());
      let flow_err = move |err| FlowError::NodeError(path.pop().unwrap(), Arc::new(err));
      let hasher = &mut state.write().await.hasher;
      node.run(&node_dir, hasher).await.map_err(flow_err)
    };

    tokio::spawn(node_fut)
      .map(|res| match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(node_run_error)) => Err(node_run_error),
        Err(join_error) => Err(join_error.into()),
      })
      .boxed()
      .shared()
  }
}

#[derive(Clone, Debug)]
pub enum FlowError {
  PanicOrAbort(Arc<JoinError>),
  NodeNotFound(String),
  CircularDependency(Vec<String>),
  RequiringOwnPath(String),
  NodeError(String, Arc<NodeError>),
}
impl From<JoinError> for FlowError {
  fn from(error: JoinError) -> Self {
    Self::PanicOrAbort(Arc::new(error))
  }
}
