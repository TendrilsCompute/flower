use std::{io::IsTerminal, process::ExitCode};

use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, Subcommand};
use flower::flow::{FlowDescription, FlowError, FlowRunner, RunConfig};
use flower_construct::{
  ConstructionError,
  path::{canonicalize_on, current_dir, find_file_in_parents},
};
use flower_foss::{flow::system_verilog_flows, project::Project};
use thiserror::Error;
use tokio::{fs, io};

#[derive(Parser)]
#[command(version)]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Run a node of a top's flow and its dependencies
  Run {
    #[command(flatten)]
    flow: FlowArgs,

    /// A set of nodes to run, rather than every node
    #[arg(short, long, required_if_eq("no_deps", "true"))]
    node: Vec<String>,

    /// The flow directory, holding a numbered directory per run,
    /// or with `--no-history`, the run directory itself
    #[arg(short, long, default_value_t = Utf8PathBuf::from("./run"))]
    dir: Utf8PathBuf,

    #[command(flatten)]
    config: RunConfig,
  },
  /// Export a top's flow to a file (`flow.msgpackz`), to be run with `flower run`
  Export {
    #[command(flatten)]
    flow: FlowArgs,

    /// The file to export to
    out: Utf8PathBuf,
  },
}

/// What a top's flow is built from.
#[derive(Args)]
struct FlowArgs {
  /// Top level
  top_level: String,

  /// Path to project description path
  #[arg(short, long)]
  project_file: Option<Utf8PathBuf>,

  /// Cocotb test parameter file, each run in a test of its own.
  /// A directory stands for the `.json` files in it.
  #[arg(short, long)]
  test_params: Vec<Utf8PathBuf>,
}

#[tokio::main(flavor = "local")]
async fn main() -> ExitCode {
  let stderr = std::io::stderr;
  tracing_subscriber::fmt().with_writer(stderr).with_ansi(stderr().is_terminal()).init();

  match run(Cli::parse().command).await {
    Ok(()) => ExitCode::SUCCESS,
    Err(PetalError::Flow(err)) => {
      eprintln!("Flow error: {}", err.display_with_details().await);
      ExitCode::FAILURE
    }
    Err(err) => {
      eprintln!("{err}");
      ExitCode::FAILURE
    }
  }
}

async fn run(command: Command) -> PetalResult<()> {
  let current_dir = current_dir()?;
  match command {
    Command::Run { flow, node, dir, config } => {
      let flow = build_flow(flow, &current_dir).await?;
      FlowRunner::run(flow, &dir, &node, config).await?;
    }
    Command::Export { flow, out } => {
      let flow = build_flow(flow, &current_dir).await?;
      flow.export(&current_dir.join(out)).await?;
    }
  }

  Ok(())
}

async fn build_flow(args: FlowArgs, current_dir: &Utf8Path) -> PetalResult<FlowDescription> {
  let FlowArgs { top_level, mut project_file, test_params } = args;

  let project = {
    if project_file.is_none() {
      project_file = find_file_in_parents(current_dir, "petal.toml").await?
    }
    let Some(mut path) = project_file else { return Err(PetalError::NoProjectFileFound) };

    let project_source =
      fs::read_to_string(&path).await.map_err(|e| PetalError::IoError(path.clone(), e))?;

    path.pop();
    let path = canonicalize_on(&path, current_dir).await?;
    toml::from_str::<Project>(&project_source)?.with_paths_canonicalized_on(&path).await?
  };

  let mut canonical_test_params = Vec::new();
  for path in &test_params {
    canonical_test_params.push(canonicalize_on(path, current_dir).await?);
  }

  Ok(system_verilog_flows(&top_level, &project, &canonical_test_params)?)
}

pub type PetalResult<T> = Result<T, PetalError>;
#[derive(Error, Debug)]
pub enum PetalError {
  #[error("Io error involving {0}: {1}")]
  IoError(Utf8PathBuf, io::Error),
  #[error("Flow construction error: {0}")]
  Construction(#[from] ConstructionError),
  #[error("Flow error: {0}")]
  Flow(#[from] FlowError),
  #[error("No project file found")]
  NoProjectFileFound,
  #[error("Project deserialization error: {0}")]
  ProjectDeserialization(#[from] toml::de::Error),
}
