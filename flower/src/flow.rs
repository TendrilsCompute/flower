pub mod command;
pub mod node;
pub mod path;

use camino::{Utf8Path, Utf8PathBuf};
use futures::future::{BoxFuture, FutureExt, Shared, join_all};
use std::{
  collections::{BTreeMap, BTreeSet},
  future,
  sync::Arc,
};
use thiserror::Error;
use tokio::{fs, io, sync::Mutex, task::JoinError};
use tracing::{debug, info};

use crate::util::hash::PathHasher;
use node::{Node, NodeError};

/// The nodes of a flow.
#[derive(Default)]
pub struct FlowDescription {
  pub nodes: BTreeMap<String, Node>,
}

type NodeId = String;
pub type FlowTask = Shared<BoxFuture<'static, FlowResult<()>>>;

struct RunContext {
  flow_dir: Utf8PathBuf,
  run_dir: Utf8PathBuf,
  /// Earlier run directories, latest first.
  previous_runs: Vec<Utf8PathBuf>,
  force_rerun: bool,
  hasher: Mutex<PathHasher>,
}

pub struct FlowRunner {
  nodes: BTreeMap<NodeId, Arc<Node>>,
  context: Arc<RunContext>,
  spawned: BTreeMap<NodeId, FlowTask>,
  spawning: BTreeSet<NodeId>,
}

impl FlowRunner {
  pub async fn run(
    flow: FlowDescription,
    flow_dir: Utf8PathBuf,
    name: &str,
    force_rerun: bool,
  ) -> FlowResult<Utf8PathBuf> {
    let (run_dir, previous_runs) = Self::new_run_dir(&flow_dir).await?;
    info!("Running in {run_dir}.");

    let context =
      RunContext { flow_dir, run_dir, previous_runs, force_rerun, hasher: Default::default() };
    let mut runner = Self {
      nodes: flow.nodes.into_iter().map(|(name, node)| (name, Arc::new(node))).collect(),
      context: Arc::new(context),
      spawned: BTreeMap::new(),
      spawning: BTreeSet::new(),
    };

    let result = runner.spawn_node(name).await;
    // Let the other nodes finish in the case of an error.
    join_all(runner.spawned.values().cloned()).await;

    result.map(|()| runner.context.run_dir.clone())
  }

  /// Creates the next numbered run directory and points `latest` at it.
  /// Returns it and the earlier run directories, latest first.
  async fn new_run_dir(flow_dir: &Utf8Path) -> FlowResult<(Utf8PathBuf, Vec<Utf8PathBuf>)> {
    fs::create_dir_all(flow_dir).await.map_err(io_err(flow_dir))?;

    let mut number = 0;
    let mut prev_runs = Vec::new();
    let mut entries = fs::read_dir(flow_dir).await.map_err(io_err(flow_dir))?;
    while let Some(entry) = entries.next_entry().await.map_err(io_err(flow_dir))? {
      let name = entry.file_name();
      let Some(name) = name.to_str() else { continue };
      if !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_digit())
        && let Ok(prev_number) = name.parse::<u32>()
      {
        number = u32::max(number, prev_number + 1);
        prev_runs.push(flow_dir.join(name))
      }
    }
    prev_runs.sort_unstable_by(|a, b| b.cmp(a));

    let run_name = format!("{number:03}");
    let run_dir = flow_dir.join(&run_name);
    fs::create_dir(&run_dir).await.map_err(io_err(&run_dir))?;

    let latest = flow_dir.join("latest");
    match fs::remove_file(&latest).await {
      Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(io_err(&latest)(err)),
      _ => {}
    }
    fs::symlink(&run_name, &latest).await.map_err(io_err(&latest))?;

    Ok((run_dir, prev_runs))
  }

  /// Recursively spawns the node and its dependencies, returning a shared future.
  fn spawn_node(&mut self, name: &str) -> FlowTask {
    if let Some(task) = self.spawned.get(name) {
      return task.clone();
    }

    let flow_err = |err| future::ready(Err(err)).boxed().shared();
    let node_err = |err| flow_err(FlowError::NodeError(name.to_string(), Arc::new(err)));

    let Some(node) = self.nodes.get(name).cloned() else {
      return flow_err(FlowError::NodeNotFound(name.into()));
    };
    // Checked before the node is marked as spawning, which an early return would leave it as.
    if let Err(err) = node.check_paths(&self.context.flow_dir) {
      return node_err(err);
    }
    let dep_names = match node.deps() {
      Ok(names) => names,
      Err(err) => return node_err(err),
    };
    if dep_names.contains(name) {
      return node_err(NodeError::RequiringOwnPath);
    }

    if !self.spawning.insert(name.into()) {
      return flow_err(FlowError::CircularDependency(name.into()));
    }
    debug!("{name} depends on {dep_names:?}.");
    let dep_tasks: Vec<_> = dep_names.iter().map(|dep| self.spawn_node(dep)).collect();

    let node_fut = {
      let (context, name) = (self.context.clone(), name.to_string());
      async move {
        for dep in dep_tasks {
          dep.await?;
        }
        context.run_node(&name, &node).await
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

impl RunContext {
  async fn run_node(&self, name: &str, node: &Node) -> FlowResult<()> {
    let node_err = |err: NodeError| FlowError::NodeError(name.to_string(), err.into());
    let node_dir = self.run_dir.join(name);

    if !self.force_rerun
      && let Some(dir) = self.previous_node_dir(name).await?
    {
      let rerun_required = {
        let previous_dir = self.flow_dir.join(&dir);
        let mut hasher = self.hasher.lock().await;
        node.rerun_required(&previous_dir, &node_dir, &mut hasher).await.map_err(node_err)?
      };
      if !rerun_required {
        info!("Skipping {name}.");
        // Relative, so the flow directory can be moved.
        fs::symlink(Utf8Path::new("..").join(&dir), &node_dir).await.map_err(io_err(&node_dir))?;
        return Ok(());
      }
    }

    info!("Running {name}.");
    fs::create_dir(&node_dir).await.map_err(io_err(&node_dir))?;
    node.run(&node_dir, &self.hasher).await.map_err(node_err)
  }

  /// `name`'s directory in the run it last ran in, relative to the flow directory,
  /// e.g. `002/<name>`. It's found through the latest earlier run with an entry for it.
  async fn previous_node_dir(&self, name: &str) -> FlowResult<Option<Utf8PathBuf>> {
    for prev_dir in &self.previous_runs {
      let entry = prev_dir.join(name);
      let Ok(metadata) = fs::symlink_metadata(&entry).await else { continue };
      if !metadata.is_symlink() {
        return Ok(Some(Utf8Path::new(prev_dir.file_name().unwrap()).join(name)));
      }
      // Links are always `../<run>/<name>`, to where the node ran.
      let target = fs::read_link(&entry).await.map_err(io_err(&entry))?;
      let dir = Utf8PathBuf::from_path_buf(target)
        .ok()
        .and_then(|target| target.strip_prefix("..").ok().map(Utf8Path::to_owned))
        .ok_or_else(|| FlowError::InvalidLink(entry.clone()))?;
      return Ok(Some(dir));
    }
    Ok(None)
  }
}

fn io_err(path: impl Into<Utf8PathBuf>) -> impl FnOnce(io::Error) -> FlowError {
  let path = path.into();
  move |err| FlowError::Io(path, Arc::new(err))
}

type FlowResult<T> = Result<T, FlowError>;
#[derive(Clone, Debug, Error)]
pub enum FlowError {
  #[error("Async task didn't complete: {0}")]
  PanicOrAbort(Arc<JoinError>),
  #[error("Node '{0}' could not be found.")]
  NodeNotFound(String),
  #[error("Circular dependency involving node '{0}'.")]
  CircularDependency(String),
  #[error("Node '{0}' failure: {1}")]
  NodeError(String, Arc<NodeError>),
  #[error("Io error involving {0}: {1}")]
  Io(Utf8PathBuf, Arc<io::Error>),
  #[error("{0} isn't a link to `../<run>/<node>`")]
  InvalidLink(Utf8PathBuf),
}
impl From<JoinError> for FlowError {
  fn from(error: JoinError) -> Self {
    Self::PanicOrAbort(Arc::new(error))
  }
}
