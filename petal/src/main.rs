use std::{env, io::ErrorKind};

use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;
use flower::flow::FlowRunner;
use flower_foss::{flow::system_verilog_flows, project::Project};
use tokio::fs;

#[derive(Parser)]
struct Cli {
  project_file: Utf8PathBuf,

  node_name: String,

  /// Run directory
  #[arg(short, long, default_value_t = Utf8PathBuf::from("./run"))]
  dir: Utf8PathBuf,
}

fn make_absolute(path: &Utf8Path, root: &Utf8Path) -> Utf8PathBuf {
  if path.is_relative() { root.join(path) } else { path.into() }
}

#[tokio::main(flavor = "local")]
async fn main() -> Result<()> {
  let cli = Cli::parse();

  tracing_subscriber::fmt::init();

  let current_dir = Utf8PathBuf::from_path_buf(env::current_dir()?).unwrap();

  let project = {
    let project_source = fs::read_to_string(&cli.project_file)
      .await
      .with_context(|| format!("Failed to open project file: {}", cli.project_file))?;
    let path = make_absolute(cli.project_file.parent().unwrap(), &current_dir);
    toml::from_str::<Project>(&project_source)?.with_absolute_paths(&path)
  };

  let dir = &cli.dir;
  match fs::create_dir(&dir).await {
    Ok(_) => (),
    Err(e) if e.kind() == ErrorKind::AlreadyExists => (),
    err => err?,
  }
  let dir = Utf8PathBuf::from_path_buf(fs::canonicalize(&cli.dir).await?)
    .map_err(|_| anyhow!("Non UTF8 run directory."))?;

  let flow = system_verilog_flows(dir, "graft_engine", &project)?;

  FlowRunner::from(flow).spawn_node(&cli.node_name, true).await?;

  Ok(())
}
