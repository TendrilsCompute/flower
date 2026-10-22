use std::path::PathBuf;

use camino::Utf8PathBuf;
use clap::Parser;
use flower::flow::{FlowError, FlowRunner};
use flower_construct::{
  ConstructionError,
  path::{canonicalize_on, current_dir, find_file_in_parents},
};
use flower_foss::{flow::system_verilog_flows, project::Project};
use thiserror::Error;
use tokio::{fs, io};

#[derive(Parser)]
struct Cli {
  /// Top level
  top_level: String,

  /// Flow of the node
  #[arg(short, long, default_value_t = "verilator_build".to_string())]
  node: String,

  /// Path to project description path
  #[arg(short, long)]
  project_file: Option<Utf8PathBuf>,

  /// Run directory
  #[arg(short, long, default_value_t = Utf8PathBuf::from("./run"))]
  dir: Utf8PathBuf,

  /// Force rerun
  #[arg(short, long)]
  force_rerun: bool,

  /// Cocotb test parameter file, each run in a test of its own.
  /// A directory stands for the `.json` files in it.
  #[arg(short, long)]
  test_params: Vec<Utf8PathBuf>,
}

#[tokio::main(flavor = "local")]
async fn main() -> PetalResult<()> {
  let Cli { top_level, node, mut project_file, dir, force_rerun, test_params } = Cli::parse();

  tracing_subscriber::fmt::init();

  let current_dir = current_dir()?;

  let dir = {
    fs::create_dir_all(&dir).await.map_err(|e| PetalError::IoError(dir.clone(), e))?;
    canonicalize_on(&dir, &current_dir).await?
  };

  let project = {
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

  let mut canonical_test_params = Vec::new();
  for path in &test_params {
    canonical_test_params.push(canonicalize_on(path, &current_dir).await?);
  }

  let flow = system_verilog_flows(dir, &top_level, &project, &canonical_test_params)?;

  FlowRunner::from(flow).spawn_node(&node, force_rerun).await?;

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
  #[error("Flow construction error: {0}")]
  Construction(#[from] ConstructionError),
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
