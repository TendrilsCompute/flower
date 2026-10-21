use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use flower::flow::{
  command::{Arg, ExternalCommand},
  node::Node,
};
use flower_construct::{
  ConstructionResult, generate_path_list, get_executable, path::files_with_extension,
};
use serde::{Deserialize, Serialize};

use crate::sv::SystemVerilogTop;

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct VerilatorConfig;

pub struct Verilator<'a> {
  pub design: &'a SystemVerilogTop<'a>,
  pub config: VerilatorConfig,
}

impl<'a> Verilator<'a> {
  fn verilator_waiver_files(&self) -> impl Iterator<Item = Utf8PathBuf> {
    self.design.source_paths().flat_map(|l| files_with_extension(l, "vlt"))
  }

  fn base_node<'args>(
    &self,
    flist_path: impl Into<Utf8PathBuf>,
    additional_args: impl IntoIterator<Item = &'args str>,
    env: BTreeMap<String, String>,
  ) -> ConstructionResult<Node> {
    let flist_path = flist_path.into();
    let waiver_files: Vec<_> = self.verilator_waiver_files().collect();

    let args = ["--top-module", self.design.top, "-f", flist_path.as_str()]
      .into_iter()
      .chain(waiver_files.iter().map(|p| p.as_str()))
      .chain(additional_args.into_iter().map(|a| a as &str))
      .map(Arg::from)
      .collect();

    let mut required_paths = self.design.required_paths();
    required_paths.push(flist_path);
    required_paths.extend(waiver_files);

    let commands = vec![ExternalCommand { exec: get_executable("verilator")?, args, env }.into()];

    Ok(Node { required_paths, commands })
  }

  pub fn lint(&self, flist_path: impl Into<Utf8PathBuf>) -> ConstructionResult<Node> {
    self.base_node(flist_path, ["--lint-only"], Default::default())
  }

  pub fn build<'args>(
    &self,
    flist_path: impl Into<Utf8PathBuf>,
    additional_args: impl IntoIterator<Item = &'args str>,
  ) -> ConstructionResult<Node> {
    let env = BTreeMap::from([(
      "PATH".to_string(),
      generate_path_list(["make", "g++", "uname", "sh", "python3", "xargs"])?,
    )]);
    self.base_node(flist_path, ["--binary"].into_iter().chain(additional_args), env)
  }
}
