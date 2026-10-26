use std::{
  io::{self, Write},
  process::ExitCode,
};
use tokio::fs;

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};

use flower::{
  flow::{FlowDescription, FlowRunner, RunConfig},
  util::hash::PathHasher,
};

#[derive(Parser)]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Output (as raw bytes) the hash of a path
  Hash { path: Utf8PathBuf },
  /// Write the dependencies of an exported flow's nodes as TOML:
  /// a `[<node>]` table for each node, holding `deps = [<node>…]`
  Deps {
    /// The exported flow
    flow: Utf8PathBuf,
    /// Where to write the TOML, rather than stdout
    #[arg(long)]
    out: Option<Utf8PathBuf>,
  },
  /// Run a node of an exported flow and its dependencies, as petal does
  Run {
    /// The exported flow
    #[arg(long)]
    flow: Utf8PathBuf,
    /// A set of nodes to run, rather than every node
    #[arg(short, long, required_if_eq("no_deps", "true"))]
    node: Vec<String>,
    /// The flow directory, holding a numbered directory per run,
    /// or with `--no-history`, the run directory itself
    #[arg(short, long, default_value = "./run")]
    dir: Utf8PathBuf,
    #[command(flatten)]
    config: RunConfig,
  },
}

#[tokio::main(flavor = "local")]
async fn main() -> ExitCode {
  tracing_subscriber::fmt::init();

  match run(Cli::parse().command).await {
    Ok(()) => ExitCode::SUCCESS,
    Err(err) => {
      eprintln!("{err}");
      ExitCode::FAILURE
    }
  }
}

async fn run(command: Command) -> Result<(), String> {
  fn to_str(err: impl ToString) -> String {
    err.to_string()
  }
  match command {
    Command::Hash { path } => {
      let hash = PathHasher::default().hash(&path).await.map_err(to_str)?;
      let mut stdout = io::stdout().lock();
      stdout
        .write_all(&hash)
        .and_then(|()| stdout.flush())
        .map_err(|err| format!("stdout: {err}"))?;
    }
    Command::Deps { flow, out } => {
      let flow = FlowDescription::read(&flow).await.map_err(to_str)?;
      let toml = flow.deps_toml().map_err(to_str)?;
      match out {
        Some(path) => fs::write(&path, toml).await.map_err(|err| format!("{path}: {err}"))?,
        None => print!("{toml}"),
      }
    }
    Command::Run { flow, node, dir, config } => {
      let flow = FlowDescription::read(&flow).await.map_err(to_str)?;
      FlowRunner::run(flow, &dir, &node, config).await.map_err(to_str)?;
    }
  }
  Ok(())
}
