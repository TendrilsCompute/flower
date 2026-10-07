use camino::{Utf8Path, Utf8PathBuf};
use flower::add_internal_command;
use flower::flow::{
  command::{Command, CommandResult, ExternalCommand, InternalCommands},
  node::Node,
};
use std::collections::BTreeSet;
use tokio::{
  fs::File,
  io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
};

use crate::{FlowBuildResult, get_executable, sv::SystemVerilogTop};

pub struct Slang<'a> {
  pub design: &'a SystemVerilogTop<'a>,
}
impl<'a> From<&'a SystemVerilogTop<'a>> for Slang<'a> {
  fn from(design: &'a SystemVerilogTop<'a>) -> Self {
    Self { design }
  }
}

impl<'a> Slang<'a> {
  fn depfiles(&self) -> FlowBuildResult<Command> {
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
      .collect();

    Ok(ExternalCommand { exec: get_executable("slang")?, args, ..Default::default() }.into())
  }

  pub fn flist(&self, builtins: &mut InternalCommands) -> FlowBuildResult<Node> {
    add_internal_command!(builtins, "flist_from_slang", flist_from_slang);
    let commands = vec![self.depfiles()?, Command::internal("flist_from_slang")];
    Ok(Node { required_paths: self.design.required_paths(), commands })
  }

  pub fn lint(&self, flist_path: impl Into<Utf8PathBuf>) -> FlowBuildResult<Node> {
    let flist_path = flist_path.into();

    let args = ["--top", self.design.top, "-f", flist_path.as_str()]
      .into_iter()
      .map(str::to_string)
      .collect();

    let mut required_paths = self.design.required_paths();
    required_paths.push(flist_path);

    let commands =
      vec![ExternalCommand { exec: get_executable("slang")?, args, ..Default::default() }.into()];

    Ok(Node { required_paths, commands })
  }
}

async fn get_includes(
  includes_file: impl AsRef<Utf8Path>,
) -> Result<BTreeSet<Utf8PathBuf>, &'static str> {
  let include_read_err = |_| "Failed to read includes.f";
  let mut includes =
    File::open(includes_file.as_ref()).await.map_err(include_read_err).map(BufReader::new)?.lines();

  let mut include_dirs = BTreeSet::new();
  while let Some(file) = includes.next_line().await.map_err(include_read_err)? {
    let mut dir = Utf8PathBuf::from(file);
    dir.pop();
    include_dirs.insert(dir);
  }
  Ok(include_dirs)
}

pub async fn flist_from_slang(node_dir: &Utf8Path) -> CommandResult {
  let body = async move {
    let write_err = |_| "Failed to write to top.f";

    let mut output = File::create(node_dir.join("top.f"))
      .await
      .map_err(|_| "Failed to read top.f")
      .map(BufWriter::new)?;

    for include_dir in get_includes(node_dir.join("includes.f")).await?.into_iter() {
      output.write_all(b"+incdir+").await.map_err(write_err)?;
      output.write_all(include_dir.as_str().as_bytes()).await.map_err(write_err)?;
      output.write_all(b"\n").await.map_err(write_err)?;
    }

    let mut modules =
      File::open(node_dir.join("modules.f")).await.map_err(|_| "Failed to open modules.f")?;

    io::copy(&mut modules, &mut output).await.map_err(|_| "Couldn't append modules.f to top.f")?;

    output.flush().await.map_err(write_err)?;
    Result::<(), &'static str>::Ok(())
  };

  match body.await {
    Ok(()) => CommandResult::default(),
    Err(message) => CommandResult { stderr: message.into(), stdout: Default::default(), status: 1 },
  }
}
