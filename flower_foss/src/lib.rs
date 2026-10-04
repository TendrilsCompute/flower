use std::collections::BTreeMap;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use flower::{exec::ExecRunInfo, flow::FlowDescription, node::Node};
use serde::{Deserialize, Serialize};
use which::which;

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

fn is_lexically_canonical(path: &Utf8Path) -> bool {
  if !path.is_absolute() {
    return false;
  }
  !path.components().any(|comp| matches!(comp, Utf8Component::CurDir | Utf8Component::ParentDir))
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

pub fn lint(dir: impl Into<Utf8PathBuf>, top: &str, project: &Project) -> FlowDescription {
  let nodes = BTreeMap::from([(
    "slang_lint".to_string(),
    slang_lint(top, project.sv_lib.values().next().unwrap()),
  )]);

  FlowDescription { dir: dir.into(), nodes }
}

pub fn slang_lint(top: &str, sv: &SystemVerilogLibrary) -> Node {
  let exec = Utf8PathBuf::from_path_buf(which("slang").unwrap()).unwrap();

  let required_paths = sv.paths().cloned().collect();

  let inc_args = sv.includes.iter().map(|inc_dir| format!("-I{}", inc_dir.as_str()));

  let src_args = sv.sources.iter().map(|src_dir| src_dir.clone().into_string() + ".../*.sv");

  let mut args: Vec<String> = inc_args.chain(src_args).collect();

  let additional_args =
    ["--top", top, "--depfile-trim", "--Mmodule", "modules.f", "--Minclude", "includes.f"];
  args.extend(additional_args.into_iter().map(str::to_string));

  Node { required_paths, execs: vec![ExecRunInfo { exec, args, ..Default::default() }] }
}
