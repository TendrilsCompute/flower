use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use flower::flow::{FlowDescription, node::Node};
use flower_construct::ConstructionResult;

use crate::{
  project::Project,
  sv::SystemVerilogTop,
  tool::{cocotb, slang::Slang, verilator::Verilator},
};

pub fn system_verilog_flows(
  dir: impl Into<Utf8PathBuf>,
  top: &str,
  project: &Project,
) -> ConstructionResult<FlowDescription> {
  let dir = dir.into();
  let design = SystemVerilogTop { top, libs: project.sv_lib.values().collect() };
  let flist_file = dir.join("flist/top.f");
  let cocotb_args_file = dir.join("cocotb_config").join(cocotb::VERILATOR_ARGS_FILE);

  let collector = Node {
    required_paths: vec![
      dir.join("slang_lint/run.msgpackz"),
      dir.join("verilator_lint/run.msgpackz"),
    ],
    commands: vec![],
  };

  let slang = Slang::from(&design);
  let verilator = Verilator { design: &design, config: Default::default() };

  let nodes = BTreeMap::from([
    ("flist".to_string(), slang.flist()?),
    ("slang_lint".to_string(), slang.lint(&flist_file)?),
    ("verilator_lint".to_string(), verilator.lint(&flist_file)?),
    ("all_lints".to_string(), collector),
    ("verilator_build".to_string(), verilator.build(&flist_file, [])?),
    ("cocotb_config".to_string(), cocotb::config()?),
    (
      "verilator_cocotb_build".to_string(),
      verilator.cocotb_build(flist_file, cocotb_args_file, [])?,
    ),
  ]);

  Ok(FlowDescription { dir, nodes })
}
