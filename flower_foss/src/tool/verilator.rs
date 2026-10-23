use camino::Utf8PathBuf;
use flower::flow::{
  command::{Arg, Env, ExternalCommand},
  node::Node,
};
use flower_construct::{
  ConstructionResult, env_vars, flow::NodeRef, generate_path_list, get_executable,
  path::files_with_extension,
};
use serde::{Deserialize, Serialize};

use crate::{sv::SystemVerilogTop, tool::cocotb};

/// The `--prefix` of the simulator [`Verilator::cocotb_build`] builds, which names its executable.
const COCOTB_SIM_PREFIX: &str = "Vtop";
/// Verilator's default output directory.
const OBJ_DIR: &str = "obj_dir";

/// The simulator built by the [`Verilator::cocotb_build`] node `build`.
pub fn cocotb_sim(build: &NodeRef) -> Utf8PathBuf {
  build.file(OBJ_DIR).join(COCOTB_SIM_PREFIX)
}

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

  fn base_node(
    &self,
    flist_path: impl Into<Utf8PathBuf>,
    additional_args: impl IntoIterator<Item = Arg>,
    env: Env,
  ) -> ConstructionResult<Node> {
    let flist_path = flist_path.into();
    let waiver_files: Vec<_> = self.verilator_waiver_files().collect();

    let args = ["--top-module", self.design.top, "-f", flist_path.as_str()]
      .into_iter()
      .chain(waiver_files.iter().map(|p| p.as_str()))
      .map(Arg::from)
      .chain(additional_args)
      .collect();

    let mut required_paths = self.design.required_paths();
    required_paths.push(flist_path);
    required_paths.extend(waiver_files);

    let commands =
      vec![ExternalCommand { exec: get_executable("verilator")?, args, env, stdout: None }.into()];

    Ok(Node { required_paths, commands })
  }

  pub fn lint(&self, flist_path: impl Into<Utf8PathBuf>) -> ConstructionResult<Node> {
    self.base_node(flist_path, [Arg::from("--lint-only")], Default::default())
  }

  fn build_env() -> ConstructionResult<Env> {
    let path = generate_path_list(["make", "g++", "uname", "sh", "python3", "xargs"])?;
    Ok(env_vars([("PATH", path.into())]).collect())
  }

  pub fn build<'args>(
    &self,
    flist_path: impl Into<Utf8PathBuf>,
    additional_args: impl IntoIterator<Item = &'args str>,
  ) -> ConstructionResult<Node> {
    let args = ["--binary"].into_iter().chain(additional_args).map(Arg::from);
    self.base_node(flist_path, args, Self::build_env()?)
  }

  /// Builds the [`cocotb_sim`] for cocotb to drive through VPI,
  /// linking in cocotb as configured by the [`cocotb::config`] node.
  pub fn cocotb_build<'args>(
    &self,
    flist_path: impl Into<Utf8PathBuf>,
    cocotb_config: &NodeRef,
    additional_args: impl IntoIterator<Item = &'args str>,
  ) -> ConstructionResult<Node> {
    let cocotb_args_file = cocotb_config.file(cocotb::VERILATOR_ARGS_FILE);
    let args =
      ["--cc", "--exe", "--build", "--vpi", "--public-flat-rw", "--prefix", COCOTB_SIM_PREFIX]
        .into_iter()
        .chain(additional_args)
        .map(Arg::from)
        .chain([Arg::Lines(cocotb_args_file.clone())]);

    let mut node = self.base_node(flist_path, args, Self::build_env()?)?;
    node.required_paths.push(cocotb_args_file);
    Ok(node)
  }
}
