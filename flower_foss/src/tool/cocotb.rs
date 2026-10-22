use flower::flow::{
  command::{Arg, Command, ExternalCommand, Extract, TextPart},
  node::Node,
};
use flower_construct::{ConstructionResult, get_executable};

/// The file in the node's directory holding the arguments that build a cocotb verilator simulator.
pub const VERILATOR_ARGS_FILE: &str = "verilator.args";

/// Queries cocotb-config for the VPI library verilator links against
/// and writes the verilator arguments that link it in and use cocotb's main.
pub fn config() -> ConstructionResult<Node> {
  let exec = get_executable("cocotb-config")?;
  let cocotb_config = |args: &[&str], stdout: &str| -> Command {
    ExternalCommand {
      exec: exec.clone(),
      args: args.iter().copied().map(Arg::from).collect(),
      stdout: Some(stdout.into()),
      ..Default::default()
    }
    .into()
  };

  // The library file is like "libcocotbvpi_verilator.so", but -l expects "cocotbvpi_verilator".
  let link_name = Extract::new(r"/lib([^/\s]+)\.so\s*$", "$1").expect("The regex is valid.");

  let verilator_args = vec![
    "-LDFLAGS\n-Wl,-rpath,".into(),
    TextPart::file("libs_dir"),
    " -L".into(),
    TextPart::file("libs_dir"),
    " -l".into(),
    TextPart::Match { path: "vpi_lib".into(), extract: link_name },
    "\n".into(),
    TextPart::file("share_dir"),
    "/lib/verilator/verilator.cpp\n".into(),
  ];

  let commands = vec![
    cocotb_config(&["--libs"], "libs_dir"),
    cocotb_config(&["--lib-name-path", "vpi", "verilator"], "vpi_lib"),
    cocotb_config(&["--share"], "share_dir"),
    Command::write(VERILATOR_ARGS_FILE, verilator_args),
  ];

  Ok(Node { required_paths: vec![], commands })
}
