use camino::Utf8PathBuf;
use flower::flow::{
  command::{Command, ExternalCommand, Extract, FromFile, MatchCount, Value},
  node::Node,
};
use flower_construct::{ConstructionResult, get_executable};

use crate::sv::SystemVerilogTop;

/// The file list [`Slang::flist`] writes in its node's directory.
pub const FLIST_FILE: &str = "top.f";

pub struct Slang<'a> {
  pub design: &'a SystemVerilogTop<'a>,
}
impl<'a> From<&'a SystemVerilogTop<'a>> for Slang<'a> {
  fn from(design: &'a SystemVerilogTop<'a>) -> Self {
    Self { design }
  }
}

impl<'a> Slang<'a> {
  pub fn slang_param_args(&self) -> impl Iterator<Item = Value> {
    self.design.param_overrides().flat_map(|param| ["-G".to_string(), param]).map(Value::from)
  }

  fn depfiles(&self) -> ConstructionResult<Command> {
    let args = self
      .design
      .include_paths()
      .map(|p| format!("-I{p}"))
      .chain(self.design.source_paths().map(|p| format!("{p}/.../*.sv")))
      .chain(
        [
          "--top",
          self.design.top,
          "--depfile-trim",
          "--Mmodule",
          "modules.f",
          "--Minclude",
          "includes.f",
        ]
        .into_iter()
        .map(str::to_string),
      )
      .map(Value::from)
      .chain(self.slang_param_args())
      .collect();

    Ok(ExternalCommand { exec: get_executable("slang")?, args, ..Default::default() }.into())
  }

  /// Writes [`FLIST_FILE`], the files the top depends on, from slang's depfiles.
  pub fn flist(&self) -> ConstructionResult<Node> {
    // An include directory for each header used.
    let include_dirs =
      FromFile::new("includes.f", Extract::new(r"(?mR)^(.*)/[^/]*$", "+incdir+$1\n")?)
        .count(MatchCount::ZeroOrMore)
        .distinct();
    let top_f = vec![include_dirs.into(), FromFile::contents("modules.f").into(), "\n".into()];
    let commands = vec![self.depfiles()?, Command::write(FLIST_FILE, top_f)];
    Ok(Node { required_paths: self.design.required_paths(), commands })
  }

  pub fn lint(&self, flist_path: impl Into<Utf8PathBuf>) -> ConstructionResult<Node> {
    let flist_path = flist_path.into();

    let args = ["--top", self.design.top, "-f", flist_path.as_str()]
      .into_iter()
      .map(Value::from)
      .chain(self.slang_param_args())
      .collect();

    let mut required_paths = self.design.required_paths();
    required_paths.push(flist_path);

    let commands =
      vec![ExternalCommand { exec: get_executable("slang")?, args, ..Default::default() }.into()];

    Ok(Node { required_paths, commands })
  }
}
