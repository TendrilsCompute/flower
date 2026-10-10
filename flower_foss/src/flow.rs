use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use flower::flow::{FlowDescription, command::InternalCommands, node::Node};
use flower::internal_commands;
use flower_construct::ConstructionResult;

use crate::{
  project::Project,
  sv::SystemVerilogTop,
  tool::{
    slang::{Slang, flist_from_slang},
    verilator::Verilator,
  },
};

pub fn system_verilog_flows(
  dir: impl Into<Utf8PathBuf>,
  top: &str,
  project: &Project,
) -> ConstructionResult<FlowDescription> {
  let dir = dir.into();
  let design = SystemVerilogTop { top, libs: project.sv_lib.values().collect() };
  let flist_file = dir.join("flist/top.f");

  let collector = Node {
    required_paths: vec![
      dir.join("slang_lint/run.msgpack"),
      dir.join("verilator_lint/run.msgpack"),
    ],
    commands: vec![],
  };

  let slang = Slang::from(&design);
  let verilator = Verilator { design: &design, config: Default::default() };

  let mut internal_cmds = InternalCommands::default();
  let nodes = BTreeMap::from([
    ("flist".to_string(), slang.flist(&mut internal_cmds)?),
    ("slang_lint".to_string(), slang.lint(&flist_file)?),
    ("verilator_lint".to_string(), verilator.lint(&flist_file)?),
    ("all_lints".to_string(), collector),
    ("verilator_build".to_string(), verilator.build(flist_file)?),
  ]);

  Ok(FlowDescription {
    dir,
    nodes,
    internal_cmds: internal_commands!("flist_from_slang" => flist_from_slang),
  })
}
