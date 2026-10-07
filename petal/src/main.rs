mod path;

use std::path::PathBuf;

use camino::Utf8PathBuf;
use clap::Parser;
use flower::flow::{FlowError, FlowRunner};
use flower_foss::{FlowBuildError, flow::system_verilog_flows, project::Project};
use thiserror::Error;
use tokio::{fs, io};

use crate::path::{canonicalize_on, current_dir, find_file_in_parents};

#[derive(Parser)]
struct Cli {
  node_name: String,

  #[arg(short, long)]
  project_file: Option<Utf8PathBuf>,

  /// Run directory
  #[arg(short, long, default_value_t = Utf8PathBuf::from("./run"))]
  dir: Utf8PathBuf,
}

#[tokio::main(flavor = "local")]
async fn main() -> PetalResult<()> {
  let Cli { node_name, mut project_file, dir } = Cli::parse();

  tracing_subscriber::fmt::init();

  let project = {
    let current_dir = current_dir()?;

    if project_file.is_none() {
      project_file = find_file_in_parents(&current_dir, "petal.toml").await?
    }
    let Some(mut path) = project_file else { return Err(PetalError::NoProjectFileFound) };

    let project_source =
      fs::read_to_string(&path).await.map_err(|e| PetalError::IoError(path.clone(), e))?;

    path.pop();
    let path = canonicalize_on(&path, &current_dir).await?;
    toml::from_str::<Project>(&project_source)?.with_paths_canonicalized_on(&path)
  };

  let flow = system_verilog_flows(dir, "graft_engine", &project)?;

  FlowRunner::from(flow).spawn_node(&node_name, true).await?;

  Ok(())
}

pub type PetalResult<T> = Result<T, PetalError>;
#[derive(Error, Debug)]
pub enum PetalError {
  #[error("Canonicalization of {0} failed: {1}")]
  Canonicalization(Utf8PathBuf, io::Error),
  #[error("Io error involving {0}: {1}")]
  IoError(Utf8PathBuf, io::Error),
  #[error("Non-utf8 path: {0}")]
  NonUtf8Path(PathBuf),
  #[error("Flow build error: {0}")]
  FlowBuild(#[from] FlowBuildError),
  #[error("Flow error: {0}")]
  Flow(#[from] FlowError),
  #[error("No project file found")]
  NoProjectFileFound,
  #[error("Project deserialization error: {0}")]
  ProjectDeserialization(#[from] toml::de::Error),
}
impl From<PathBuf> for PetalError {
  fn from(path: PathBuf) -> Self {
    Self::NonUtf8Path(path)
  }
}
