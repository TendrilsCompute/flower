use camino::{Utf8Path, Utf8PathBuf};
use flower::flow::FlowDescription;
use flower_construct::{
  ConstructionResult,
  flow::{FlowBuilder, NodeRef},
};

use crate::{
  project::Project,
  sv::SystemVerilogTop,
  tool::{
    cocotb::{self, CocotbTests},
    slang::{self, Slang},
    verilator::{self, Verilator},
    yosys::Formal,
  },
};

/// Each of `test_params` is run in a cocotb test of its own with `TEST_PARAMS` set to it;
/// a directory stands for the `.json` files in it.
pub fn system_verilog_flows(
  dir: impl Into<Utf8PathBuf>,
  top: &str,
  project: &Project,
  test_params: &[Utf8PathBuf],
) -> ConstructionResult<FlowDescription> {
  let design = SystemVerilogTop { top, libs: project.sv_lib.values().collect() };
  let slang = Slang::from(&design);
  let verilator = Verilator { design: &design, config: Default::default() };
  let mut flow = FlowBuilder::new(dir);

  let flist = flow.add("flist", slang.flist()?)?;
  let flist_file = flist.file(slang::FLIST_FILE);
  let slang_lint = flow.add("slang_lint", slang.lint(&flist_file)?)?;
  let verilator_lint = flow.add("verilator_lint", verilator.lint(&flist_file)?)?;
  let lints = flow.collect("all_lints", [&slang_lint, &verilator_lint])?;
  flow.add("verilator_build", verilator.build(&flist_file, [])?)?;

  if let Some(tests) = project.cocotb.get(top) {
    cocotb_flows(&mut flow, &verilator, &flist_file, tests, test_params)?;
  }

  if let Some(config) = project.formal.get(top) {
    formal_flows(&mut flow, &Formal { design: &design, config }, &flist_file, &lints)?;
  }

  Ok(flow.build())
}

fn cocotb_flows(
  flow: &mut FlowBuilder,
  verilator: &Verilator,
  flist_file: &Utf8Path,
  tests: &CocotbTests,
  test_params: &[Utf8PathBuf],
) -> ConstructionResult<()> {
  let top = verilator.design.top;
  let config = flow.add("cocotb_config", cocotb::config()?)?;
  let build =
    flow.add("verilator_cocotb_build", verilator.cocotb_build(flist_file, &config, [])?)?;
  let sim = verilator::cocotb_sim(&build);
  let test = |test_params| cocotb::test(&sim, &config, top, tests, test_params);

  let test_param_files = cocotb::test_param_files(test_params);
  if test_param_files.is_empty() {
    flow.add("cocotb_test", test(None)?)?;
  } else {
    // A test for each parameter file, collected by `cocotb_test`.
    let mut runs = Vec::new();
    for test_params in &test_param_files {
      let name = format!("cocotb_test.{}", test_params.file_stem().unwrap_or_default());
      runs.push(flow.add(name, test(Some(test_params))?)?);
    }
    flow.collect("cocotb_test", &runs)?;
  }
  Ok(())
}

fn formal_flows(
  flow: &mut FlowBuilder,
  formal: &Formal,
  flist_file: &Utf8Path,
  lints: &NodeRef,
) -> ConstructionResult<()> {
  // Only check formally once the lints pass.
  let smt2 = flow.add_after("formal_smt2", formal.smt2(flist_file)?, [lints])?;
  let bmc = flow.add("formal_bmc", formal.check(&smt2, false)?)?;
  let induction = flow.add("formal_induction", formal.check(&smt2, true)?)?;
  flow.collect("formal", [&bmc, &induction])?;
  Ok(())
}
