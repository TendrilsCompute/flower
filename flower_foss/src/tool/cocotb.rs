use std::env;

use camino::{Utf8Path, Utf8PathBuf};
use flower::flow::{
  command::{Command, Env, ExternalCommand, Extract, FromFile, MatchCount, Value},
  node::Node,
};
use flower_construct::{
  ConstructionResult, env_vars, flow::NodeRef, get_executable, path::files_with_extension,
};
use serde::{Deserialize, Serialize};

/// The file in the node's directory holding the arguments that build a cocotb verilator simulator.
pub const VERILATOR_ARGS_FILE: &str = "verilator.args";
/// The files in the [`config`] node's directory holding the libpython
/// and Python executable a cocotb simulator starts.
const LIBPYTHON_FILE: &str = "libpython";
const PYTHON_BIN_FILE: &str = "python_bin";

#[derive(Debug, Deserialize, Serialize)]
pub struct CocotbTests {
  pub test_modules: Vec<Utf8PathBuf>,
}

/// The parameter files `paths` stand for: a directory stands for the `.json` files in it.
pub fn test_param_files(paths: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
  let mut files = Vec::new();
  for path in paths {
    if path.is_dir() {
      let mut dir_files: Vec<_> = files_with_extension(path, "json").collect();
      dir_files.sort();
      files.extend(dir_files);
    } else {
      files.push(path.clone());
    }
  }
  files
}

/// Queries cocotb-config for the VPI library verilator links against and the Python it runs,
/// writing the verilator arguments that link it in and use cocotb's main.
pub fn config() -> ConstructionResult<Node> {
  let exec = get_executable("cocotb-config")?;
  let cocotb_config = |args: &[&str], stdout: &str| -> Command {
    ExternalCommand {
      exec: exec.clone(),
      args: args.iter().copied().map(Value::from).collect(),
      stdout: Some(stdout.into()),
      ..Default::default()
    }
    .into()
  };

  // The library file is like "libcocotbvpi_verilator.so", but -l expects "cocotbvpi_verilator".
  let link_name = Extract::new(r"/lib([^/\s]+)\.so\s*$", "$1")?;

  let verilator_args = vec![
    "-LDFLAGS\n-Wl,-rpath,".into(),
    FromFile::contents("libs_dir").into(),
    " -L".into(),
    FromFile::contents("libs_dir").into(),
    " -l".into(),
    FromFile::new("vpi_lib", link_name).count(MatchCount::ExactlyOne).into(),
    "\n".into(),
    FromFile::contents("share_dir").into(),
    "/lib/verilator/verilator.cpp\n".into(),
  ];

  let commands = vec![
    cocotb_config(&["--lib-dir"], "libs_dir"),
    cocotb_config(&["--lib-name-path", "vpi", "verilator"], "vpi_lib"),
    cocotb_config(&["--share"], "share_dir"),
    Command::write(VERILATOR_ARGS_FILE, verilator_args),
    cocotb_config(&["--libpython"], LIBPYTHON_FILE),
    cocotb_config(&["--python-bin"], PYTHON_BIN_FILE),
  ];

  Ok(Node { required_paths: vec![], commands })
}

/// Runs a cocotb simulator, writing the results to `results.xml`,
/// and fails if any test failed.
/// `config` is the [`config`] node.
///
/// Python packages are found through the `PYTHONPATH` this is constructed with.
pub fn test(
  sim: &Utf8Path,
  config: &NodeRef,
  top: &str,
  tests: &CocotbTests,
  test_params: Option<&Utf8Path>,
) -> ConstructionResult<Node> {
  let module_names: Vec<_> =
    tests.test_modules.iter().filter_map(|path| path.file_stem()).collect();
  let mut python_path: Vec<String> =
    tests.test_modules.iter().filter_map(|path| path.parent()).map(|dir| dir.to_string()).collect();
  python_path.dedup();
  python_path.extend(env::var("PYTHONPATH").ok());

  let libpython = config.file(LIBPYTHON_FILE);
  let python_bin = config.file(PYTHON_BIN_FILE);

  let mut env: Env = env_vars([
    ("COCOTB_TOPLEVEL", top.into()),
    ("TOPLEVEL_LANG", "verilog".into()),
    ("COCOTB_TEST_MODULES", module_names.join(",").into()),
    ("COCOTB_RESULTS_FILE", "results.xml".into()),
    ("PYTHONPATH", python_path.join(":").into()),
    // Keeps Python's bytecode out of the source tree, which may be a required path.
    ("PYTHONPYCACHEPREFIX", "pycache".into()),
  ])
  .collect();
  env.extend(env_vars(test_params.map(|path| ("TEST_PARAMS", path.as_str().into()))));

  let mut sim_env = env.clone();
  sim_env.extend(env_vars([
    ("LIBPYTHON_LOC", Value::FromFile(libpython.clone())),
    ("PYGPI_PYTHON_BIN", Value::FromFile(python_bin.clone())),
  ]));
  let run = ExternalCommand { exec: sim.into(), env: sim_env, ..Default::default() };
  // The simulator succeeds even if tests fail.
  let check = ExternalCommand {
    exec: get_executable("python3")?,
    args: ["-m", "cocotb_tools.check_results", "results.xml"].map(Value::from).into(),
    env,
    stdout: None,
  };

  let mut required_paths = vec![sim.into(), libpython, python_bin];
  required_paths
    .extend(tests.test_modules.iter().filter_map(|path| path.parent()).map(Utf8PathBuf::from));
  required_paths.extend(test_params.map(Utf8PathBuf::from));

  Ok(Node { required_paths, commands: vec![run.into(), check.into()] })
}
