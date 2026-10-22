use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use flower::flow::{FlowDescription, node::Node};
use flower_construct::ConstructionResult;

use crate::{
  project::Project,
  sv::SystemVerilogTop,
  tool::{cocotb, slang::Slang, verilator::Verilator, yosys::Formal},
};

/// Each of `test_params` is run in a cocotb test of its own with `TEST_PARAMS` set to it;
/// a directory stands for the `.json` files in it.
pub fn system_verilog_flows(
  dir: impl Into<Utf8PathBuf>,
  top: &str,
  project: &Project,
  test_params: &[Utf8PathBuf],
) -> ConstructionResult<FlowDescription> {
  let dir = dir.into();
  let design = SystemVerilogTop { top, libs: project.sv_lib.values().collect() };
  let flist_file = dir.join("flist/top.f");
  let cocotb_config_dir = dir.join("cocotb_config");
  let cocotb_args_file = cocotb_config_dir.join(cocotb::VERILATOR_ARGS_FILE);

  let collector = Node {
    required_paths: vec![
      dir.join("slang_lint/run.msgpackz"),
      dir.join("verilator_lint/run.msgpackz"),
    ],
    commands: vec![],
  };

  let slang = Slang::from(&design);
  let verilator = Verilator { design: &design, config: Default::default() };

  let mut nodes = BTreeMap::from([
    ("flist".to_string(), slang.flist()?),
    ("slang_lint".to_string(), slang.lint(&flist_file)?),
    ("verilator_lint".to_string(), verilator.lint(&flist_file)?),
    ("all_lints".to_string(), collector),
    ("verilator_build".to_string(), verilator.build(&flist_file, [])?),
    ("cocotb_config".to_string(), cocotb::config()?),
    (
      "verilator_cocotb_build".to_string(),
      verilator.cocotb_build(&flist_file, cocotb_args_file, [])?,
    ),
  ]);

  if let Some(tests) = project.cocotb.get(top) {
    let sim = dir.join("verilator_cocotb_build/obj_dir/Vtop");
    let test = |test_params| cocotb::test(&sim, &cocotb_config_dir, top, tests, test_params);

    let test_param_files = cocotb::test_param_files(test_params);
    if test_param_files.is_empty() {
      nodes.insert("cocotb_test".to_string(), test(None)?);
    } else {
      // A test for each parameter file, collected by `cocotb_test`.
      let mut collector = Node { required_paths: vec![], commands: vec![] };
      for test_params in &test_param_files {
        let name = format!("cocotb_test.{}", test_params.file_stem().unwrap_or_default());
        collector.required_paths.push(dir.join(&name).join("run.msgpackz"));
        nodes.insert(name, test(Some(test_params))?);
      }
      nodes.insert("cocotb_test".to_string(), collector);
    }
  }

  if let Some(config) = project.formal.get(top) {
    let formal = Formal { design: &design, config };
    let smt2_dir = dir.join("formal_smt2");

    let mut smt2 = formal.smt2(&flist_file)?;
    // Only check formally once the lints pass.
    smt2.required_paths.push(dir.join("all_lints/run.msgpackz"));

    nodes.extend([
      ("formal_smt2".to_string(), smt2),
      ("formal_bmc".to_string(), formal.check(&smt2_dir, false)?),
      ("formal_induction".to_string(), formal.check(&smt2_dir, true)?),
      (
        "formal".to_string(),
        Node {
          required_paths: vec![
            dir.join("formal_bmc/run.msgpackz"),
            dir.join("formal_induction/run.msgpackz"),
          ],
          commands: vec![],
        },
      ),
    ]);
  }

  Ok(FlowDescription { dir, nodes })
}
