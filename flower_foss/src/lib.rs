use std::{collections::BTreeMap, path::PathBuf};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use flower::{
  command::{Command, CommandResult, ExternalCommand},
  flow::FlowDescription,
  internal_commands,
  node::Node,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
  fs::File,
  io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
};
use which::{Error as WhichError, which};

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
  pub sv_lib: BTreeMap<String, SystemVerilogLibrary>,
}
impl Project {
  pub fn with_absolute_paths(mut self, project_dir: &Utf8Path) -> Self {
    for path in self.sv_lib.values_mut().flat_map(SystemVerilogLibrary::paths_mut) {
      *path = project_dir.join(&path);
    }
    self
  }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SystemVerilogLibrary {
  pub includes: Vec<Utf8PathBuf>,
  pub sources: Vec<Utf8PathBuf>,
}
impl SystemVerilogLibrary {
  pub fn paths_mut(&mut self) -> impl Iterator<Item = &mut Utf8PathBuf> {
    self.includes.iter_mut().chain(self.sources.iter_mut())
  }

  pub fn paths(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.includes.iter().chain(self.sources.iter())
  }
}

pub fn lint(
  dir: impl Into<Utf8PathBuf>,
  top: &str,
  project: &Project,
) -> FlowBuildResult<FlowDescription> {
  let dir = dir.into();
  let top = SystemVerilogTop { top, libs: project.sv_lib.values().collect() };

  let nodes = BTreeMap::from([
    ("flist".to_string(), top.flist_node()?),
    ("slang_lint".to_string(), top.slang_lint_node(dir.join("flist/top.f"))?),
  ]);

  Ok(FlowDescription {
    dir,
    nodes,
    internal_cmds: internal_commands!("generate_flist" => generate_flist),
  })
}

pub fn get_executable(name: &str) -> Result<Utf8PathBuf, FlowBuildError> {
  use FlowBuildError::*;
  let path = which("slang").map_err(|e| ExecutableFindError(name.to_string(), e))?;
  Utf8PathBuf::from_path_buf(path).map_err(NonUtf8Path)
}

#[derive(Debug, Error)]
pub enum FlowBuildError {
  #[error("Failed to find executable '{0}': {1}")]
  ExecutableFindError(String, WhichError),
  #[error("Non utf8 path '{0}'")]
  NonUtf8Path(PathBuf),
}
pub type FlowBuildResult<T> = Result<T, FlowBuildError>;

struct SystemVerilogTop<'s> {
  top: &'s str,
  libs: Vec<&'s SystemVerilogLibrary>,
}
impl<'s> SystemVerilogTop<'s> {
  fn required_paths(&self) -> Vec<Utf8PathBuf> {
    self.libs.iter().flat_map(|l| l.paths()).cloned().collect()
  }

  fn include_paths(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.libs.iter().flat_map(|l| l.includes.iter())
  }

  fn source_paths(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.libs.iter().flat_map(|l| l.sources.iter())
  }

  fn slang_depfiles(&self) -> FlowBuildResult<Command> {
    let args = self
      .include_paths()
      .map(|p| format!("-I{p}"))
      .chain(self.source_paths().map(|p| format!("{p}/.../*.sv")))
      .chain(
        ["--top", self.top, "--depfile-trim", "--Mmodule", "modules.f", "--Minclude", "includes.f"]
          .into_iter()
          .map(str::to_string),
      )
      .collect();

    Ok(ExternalCommand { exec: get_executable("slang")?, args, ..Default::default() }.into())
  }

  fn slang_lint_node(&self, flist_path: impl Into<Utf8PathBuf>) -> FlowBuildResult<Node> {
    let flist_path = flist_path.into();

    let args =
      ["--top", self.top, "-f", flist_path.as_str()].into_iter().map(str::to_string).collect();

    let mut required_paths = self.required_paths();
    required_paths.push(flist_path);

    let commands = vec![ExternalCommand { exec: get_executable("slang")?, args, ..Default::default() }.into()];

    Ok(Node { required_paths, commands })
  }

  pub fn flist_node(&self) -> FlowBuildResult<Node> {
    let commands = vec![self.slang_depfiles()?, Command::internal("generate_flist")];
    Ok(Node { required_paths: self.required_paths(), commands })
  }
}

async fn generate_flist(node_dir: &Utf8Path) -> CommandResult {
  let body = async move {
    let include_read_err = |_| "Failed to read includes.f";
    let write_err = |_| "Failed to write to top.f";
    let mut includes = File::open(node_dir.join("includes.f"))
      .await
      .map_err(include_read_err)
      .map(BufReader::new)?;

    let mut output = File::create(node_dir.join("top.f"))
      .await
      .map_err(|_| "Failed to read top.f")
      .map(BufWriter::new)?;

    let prefix = b"+incdir+";
    let mut line_buffer = Vec::new();

    while includes.read_until(b'\n', &mut line_buffer).await.map_err(include_read_err)? > 0 {
      output.write_all(prefix).await.map_err(write_err)?;
      output.write_all(&line_buffer).await.map_err(write_err)?;

      line_buffer.clear();
    }

    let mut modules = File::open(node_dir.join("modules.f"))
      .await
      .map_err(|_| "Failed to open modules.f")
      .map(BufReader::new)?;

    io::copy(&mut modules, &mut output).await.map_err(|_| "")?;

    output.flush().await.map_err(write_err)?;
    Result::<(), &'static str>::Ok(())
  };

  match body.await {
    Ok(()) => CommandResult::default(),
    Err(message) => CommandResult { stderr: message.into(), stdout: Default::default(), status: 1 },
  }
}
