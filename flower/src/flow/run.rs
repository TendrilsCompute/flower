use camino::{Utf8Path, Utf8PathBuf};
use clap::Args;
use futures::future::{BoxFuture, FutureExt, Shared, join_all, try_join_all};
use std::{
  collections::{BTreeMap, BTreeSet},
  future,
  sync::Arc,
};
use tokio::{fs, io};
use tracing::{debug, info};

use super::{
  FlowDescription, FlowError, FlowResult, io_err,
  node::{Node, NodeError},
};
use crate::util::{hash::PathHasher, path};

async fn canonical_dir(dir: &Utf8Path) -> FlowResult<Utf8PathBuf> {
  fs::create_dir_all(dir).await.map_err(io_err(dir))?;
  path::canonicalize(dir).await.map_err(io_err(dir))
}

type NodeId = String;
pub type FlowTask = Shared<BoxFuture<'static, FlowResult<()>>>;

/// How a flow is run.
#[derive(Clone, Debug, Default, Args)]
pub struct RunConfig {
  /// Rerun nodes even if an earlier run's output could be used
  #[arg(short, long)]
  pub force_rerun: bool,
  /// Don't use number based history in run directory.
  #[arg(long)]
  pub no_history: bool,
  /// Use dependencies' outputs in the run directory as they are, rather than running them.
  /// Throws an error if they're not present.
  #[arg(long, requires = "no_history")]
  pub no_deps: bool,
  /// Stop at the first failure, rather than letting the other nodes finish
  #[arg(long)]
  pub fail_fast: bool,
}

struct RunContext {
  flow_dir: Utf8PathBuf,
  /// The run directory, then earlier run directories, latest first, whose outputs can be reused.
  run_dirs: Vec<Utf8PathBuf>,
  config: RunConfig,
  hasher: PathHasher,
}

pub struct FlowRunner {
  nodes: BTreeMap<NodeId, Arc<Node>>,
  context: Arc<RunContext>,
  tasks: BTreeMap<NodeId, FlowTask>,
  creating: BTreeSet<NodeId>,
}

impl FlowRunner {
  pub async fn run(
    flow: FlowDescription,
    flow_dir: &Utf8Path,
    names: &[String],
    config: RunConfig,
  ) -> FlowResult<()> {
    let flow_dir = canonical_dir(flow_dir).await?;
    let run_dirs = match config.no_history {
      true => vec![flow_dir.clone()],
      false => Self::numbered_run_dirs(&flow_dir).await?,
    };
    info!("Running in {}.", run_dirs[0]);

    let context = RunContext { flow_dir, run_dirs, config, hasher: Default::default() };
    let mut runner = Self {
      nodes: flow.nodes.into_iter().map(|(name, node)| (name, Arc::new(node))).collect(),
      context: Arc::new(context),
      tasks: BTreeMap::new(),
      creating: BTreeSet::new(),
    };

    let names = match names {
      [] => runner.nodes.keys().cloned().collect(),
      names => names.to_vec(),
    };
    let targets: Vec<_> = names.iter().map(|name| runner.node_task(name)).collect();
    join_tasks(targets, runner.context.config.fail_fast).await
  }

  /// Creates the next numbered run directory and points `latest` at it.
  /// Returns it followed by the earlier run directories, latest first.
  async fn numbered_run_dirs(flow_dir: &Utf8Path) -> FlowResult<Vec<Utf8PathBuf>> {
    let mut prev_runs = Vec::new();
    let mut entries = fs::read_dir(flow_dir).await.map_err(io_err(flow_dir))?;
    while let Some(entry) = entries.next_entry().await.map_err(io_err(flow_dir))? {
      let name = entry.file_name();
      let Some(name) = name.to_str() else { continue };
      if !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_digit())
        && let Ok(prev_number) = name.parse::<u32>()
      {
        prev_runs.push((prev_number, flow_dir.join(name)))
      }
    }
    prev_runs.sort_unstable_by(|(a, _), (b, _)| b.cmp(a));

    let number = prev_runs.first().map_or(0, |(prev_number, _)| prev_number + 1);
    let run_name = format!("{number:03}");
    let run_dir = flow_dir.join(&run_name);
    fs::create_dir(&run_dir).await.map_err(io_err(&run_dir))?;

    let latest = flow_dir.join("latest");
    match fs::remove_file(&latest).await {
      Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(io_err(&latest)(err)),
      _ => {}
    }
    fs::symlink(&run_name, &latest).await.map_err(io_err(&latest))?;

    Ok([run_dir].into_iter().chain(prev_runs.into_iter().map(|(_, dir)| dir)).collect())
  }

  /// The node's shared future, creating it, and recursively its dependencies', if needed.
  fn node_task(&mut self, name: &str) -> FlowTask {
    if let Some(task) = self.tasks.get(name) {
      return task.clone();
    }

    let flow_err = |err| future::ready(Err(err)).boxed().shared();
    let node_err = |err| flow_err(FlowError::NodeError(name.to_string(), Arc::new(err)));

    let Some(node) = self.nodes.get(name).cloned() else {
      return flow_err(FlowError::NodeNotFound(name.into()));
    };
    // Checked before the node is marked as being created, which an early return would leave it as.
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

    if !self.creating.insert(name.into()) {
      return flow_err(FlowError::CircularDependency(name.into()));
    }
    debug!("{name} depends on {dep_names:?}.");
    let dep_tasks: Vec<_> = dep_names
      .iter()
      .map(|dep| match self.context.config.no_deps {
        true => self.existing_output(dep),
        false => self.node_task(dep),
      })
      .collect();

    let node_fut = {
      let (context, name) = (self.context.clone(), name.to_string());
      async move {
        join_tasks(dep_tasks, context.config.fail_fast).await?;
        context.run_node(&name, &node).await
      }
    };
    let task = node_fut.boxed().shared();

    let name = self.creating.take(name).expect("The name is expected to be in creating.");
    self.tasks.insert(name, task.clone());

    task
  }

  fn existing_output(&self, dep: &str) -> FlowTask {
    let dep_dir = self.context.run_dir().join(dep);
    let result = if dep_dir.exists() {
      debug!("Using {dep}'s output in the run directory as it is.");
      Ok(())
    } else {
      Err(FlowError::MissingDependency(dep.into(), dep_dir))
    };
    future::ready(result).boxed().shared()
  }
}

impl RunContext {
  fn run_dir(&self) -> &Utf8Path {
    &self.run_dirs[0]
  }

  async fn run_node(&self, name: &str, node: &Node) -> FlowResult<()> {
    let node_err = |err: NodeError| FlowError::NodeError(name.to_string(), err.into());
    let node_dir = self.run_dir().join(name);

    if !self.config.force_rerun
      && let Some(latest) = self.latest_output(name).await?
    {
      let rerun_required =
        node.rerun_required(&latest, &node_dir, &self.hasher).await.map_err(node_err)?;
      if !rerun_required {
        info!("Skipping {name}.");
        // Linked to, unless it's already in place, as without history.
        if latest != node_dir {
          // Relative, so the flow directory can be moved.
          let run_relative = latest.strip_prefix(&self.flow_dir).expect("In the flow dir.");
          let target = Utf8Path::new("..").join(run_relative);
          fs::symlink(target, &node_dir).await.map_err(io_err(&node_dir))?;
        }
        return Ok(());
      }
    }

    // Without history, the previous output is in the way.
    remove_output(&node_dir).await?;
    info!("Running {name}.");
    fs::create_dir(&node_dir).await.map_err(io_err(&node_dir))?;
    node.run(&node_dir, &self.hasher).await.map_err(node_err)
  }

  async fn latest_output(&self, name: &str) -> FlowResult<Option<Utf8PathBuf>> {
    for (i, run_dir) in self.run_dirs.iter().enumerate() {
      let entry = run_dir.join(name);
      let Ok(metadata) = fs::symlink_metadata(&entry).await else { continue };
      if i == 0 || !metadata.is_symlink() {
        return Ok(Some(entry));
      }
      // Links in earlier runs are always `../<run>/<name>`, to where the node ran.
      let target = fs::read_link(&entry).await.map_err(io_err(&entry))?;
      let dir = Utf8PathBuf::from_path_buf(target)
        .ok()
        .and_then(|target| target.strip_prefix("..").ok().map(|dir| self.flow_dir.join(dir)))
        .ok_or_else(|| FlowError::InvalidLink(entry.clone()))?;
      return Ok(Some(dir));
    }
    Ok(None)
  }
}

async fn join_tasks(tasks: Vec<FlowTask>, fail_fast: bool) -> FlowResult<()> {
  match fail_fast {
    true => try_join_all(tasks).await.map(drop),
    false => join_all(tasks).await.into_iter().collect(),
  }
}

/// Removes a node's output from a run directory, a directory or a link to one, if it's there.
async fn remove_output(node_dir: &Utf8Path) -> FlowResult<()> {
  let removed = match fs::symlink_metadata(node_dir).await {
    Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
    Err(err) => Err(err),
    Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(node_dir).await,
    Ok(_) => fs::remove_file(node_dir).await,
  };
  removed.map_err(io_err(node_dir))
}
