use camino::Utf8PathBuf;
use flower::flow::{
  command::{Arg, Command, ExternalCommand},
  node::Node,
};
use flower_construct::{ConstructionResult, flow::NodeRef, generate_path_list, get_executable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::sv::SystemVerilogTop;

/// Defines the `generate_smt2` proc.
const SCRIPT: &str = include_str!("yosys/generate_smt2.tcl");
const SCRIPT_FILE: &str = "generate_smt2.tcl";
/// Sources [`SCRIPT_FILE`] and calls `generate_smt2` for the design.
const CONFIG_FILE: &str = "config.tcl";

#[derive(Debug, Deserialize, Serialize)]
pub struct FormalConfig {
  /// Property files, bound into the design. They aren't in the flist,
  /// as slang only follows instantiations.
  pub props: Vec<Utf8PathBuf>,
  /// The number of time steps to check.
  pub steps: u32,
}

pub struct Formal<'a> {
  pub design: &'a SystemVerilogTop<'a>,
  pub config: &'a FormalConfig,
}

impl<'a> Formal<'a> {
  /// The SMT2 file [`Self::smt2`] writes in its node's directory.
  pub fn smt2_file(&self) -> String {
    format!("{}.smt2", self.design.top)
  }

  /// Converts the design and its properties into an SMT2 model of their state transitions.
  pub fn smt2(&self, flist_path: impl Into<Utf8PathBuf>) -> ConstructionResult<Node> {
    let flist_path = flist_path.into();
    let top = self.design.top;

    let props: Vec<_> = self.config.props.iter().map(|path| format!("{{{path}}}")).collect();
    let config = format!(
      "source {SCRIPT_FILE}\ngenerate_smt2 {{{top}}} {{{flist_path}}} [list {}] {{{}}}\n",
      props.join(" "),
      self.smt2_file(),
    );

    let yosys = ExternalCommand {
      exec: get_executable("yosys")?,
      args: ["-c", CONFIG_FILE].map(Arg::from).into(),
      ..Default::default()
    };
    let commands = vec![
      Command::write(SCRIPT_FILE, vec![SCRIPT.into()]),
      Command::write(CONFIG_FILE, vec![config.into()]),
      yosys.into(),
    ];

    let mut required_paths = self.design.required_paths();
    required_paths.push(flist_path);
    required_paths.extend(self.config.props.iter().cloned());

    Ok(Node { required_paths, commands })
  }

  /// Checks the SMT2 model written by the [`Self::smt2`] node,
  /// with bounded model checking or, if `induction`, k-induction.
  pub fn check(&self, smt2: &NodeRef, induction: bool) -> ConstructionResult<Node> {
    let top = self.design.top;
    let smt2 = smt2.file(self.smt2_file());
    let steps = self.config.steps.to_string();

    let dumps =
      [("--dump-vcd", "vcd"), ("--dump-yw", "yw"), ("--dump-vlogtb", "v"), ("--dump-smtc", "smtc")];
    let args = induction
      .then_some("-i".into())
      .into_iter()
      .chain(["-t", &steps, "--noprogress"].map(Arg::from))
      .chain(dumps.into_iter().flat_map(|(flag, ext)| [flag.into(), format!("{top}.{ext}").into()]))
      .chain([Arg::from(smt2.as_str())])
      .collect();

    // yosys-smtbmc runs the solver, yices by default, from PATH.
    let env = BTreeMap::from([("PATH".to_string(), generate_path_list(["yices-smt2"])?.into())]);
    let check = ExternalCommand { exec: get_executable("yosys-smtbmc")?, args, env, stdout: None };

    Ok(Node { required_paths: vec![smt2], commands: vec![check.into()] })
  }
}
