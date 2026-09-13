use std::{collections::BTreeMap, io::ErrorKind, path::PathBuf};

use anyhow::Result;
use flower::{
  exec::ExecRunInfo,
  flow::{FlowDescription, FlowRunner},
  node::Node,
};
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

#[tokio::main(flavor = "local")]
async fn main() -> Result<()> {
  let dir = PathBuf::from("/tmp/build");

  match fs::create_dir(&dir).await {
    Ok(_) => (),
    Err(e) if e.kind() == ErrorKind::AlreadyExists => (),
    err => err?,
  }
  let dir = fs::canonicalize(&dir).await?;

  let nodes = BTreeMap::from([
    (
      "lists".to_string(),
      Node {
        required_paths: vec![dir.join("fingers/"), dir.join("toes/")],
        execs: vec![ExecRunInfo {
          exec: PathBuf::from("ls"),
          args: vec![dir.join("fingers/").into_os_string(), dir.join("toes/").into_os_string()],
          ..Default::default()
        }],
      },
    ),
    (
      "fingers".to_string(),
      Node {
        required_paths: vec![],
        execs: vec![ExecRunInfo {
          exec: PathBuf::from("touch"),
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
          exec: PathBuf::from("touch"),
          args: vec!["toast.txt".into()],
          ..Default::default()
        }],
      },
    ),
  ]);
  let flow = FlowDescription { dir, nodes };

  FlowRunner::from(flow).spawn_node("lists").await.unwrap();

  Ok(())
}
