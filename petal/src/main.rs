use std::{collections::BTreeMap, env, io::ErrorKind, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;
use flower::{
  exec::ExecRunInfo,
  flow::{FlowDescription, FlowRunner},
  node::Node,
  serialization,
};
use flower_foss::{Project, lint};
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize)]
pub struct FlowFile {
  pub common: CommonConfig,
  pub nodes: Vec<NodeDecl>,
}

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
  pub paths: BTreeMap<String, Vec<PathBuf>>,
  pub systemverilog: Vec<NodeDecl>,
}

#[derive(Debug, Deserialize)]
pub struct NodeDecl {
  pub name: String,
  #[serde(default)]
  pub deps: Vec<String>,
  #[serde(flatten)]
  pub kind: NodeKind,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
  SlangFlist,
  VerilatorLint,
}

#[derive(Parser)]
struct Cli {
  project_file: Utf8PathBuf,

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

  /*
  let nodes = BTreeMap::from([
    (
      "lists".to_string(),
      Node {
        required_paths: vec![dir.join("fingers/"), dir.join("toes/")],
        execs: vec![ExecRunInfo {
          exec: Utf8PathBuf::from("ls"),
          args: vec![dir.join("fingers/").into_string(), dir.join("toes/").into_string()],
          ..Default::default()
        }],
      },
    ),
    (
      "fingers".to_string(),
      Node {
        required_paths: vec![],
        execs: vec![ExecRunInfo {
          exec: Utf8PathBuf::from("touch"),
          args: vec!["bean.txt".into()],
          ..Default::default()
        }],
      },
    ),
    (
      "toes".to_string(),
      Node {
        required_paths: vec![dir.join("fingers/")],
        execs: vec![ExecRunInfo {
          exec: Utf8PathBuf::from("touch"),
          args: vec!["toast.txt".into()],
          ..Default::default()
        }],
      },
    ),
  ]);
  let flow = FlowDescription { dir, nodes };
  */

  let flow_file_path = dir.join("info.msgpack");
  let flow = lint(dir, "graft_engine", &project);

  serialization::write(&flow, &flow_file_path).await?;

  FlowRunner::from(flow).spawn_node("slang_lint").await?;

  Ok(())
}
