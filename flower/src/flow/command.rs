use camino::{Utf8Path, Utf8PathBuf};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio::{io, process};

pub type ExitStatus = i32;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum Command {
  Internal {
    name: String,
    no_check: bool,
  },
  External {
    #[serde(flatten)]
    command: ExternalCommand,
    no_check: bool,
  },
}
impl Command {
  pub fn internal(name: impl Into<String>) -> Self {
    Self::Internal { name: name.into(), no_check: false }
  }

  pub fn no_check(&self) -> bool {
    match self {
      Self::Internal { no_check, .. } | Self::External { no_check, .. } => *no_check,
    }
  }

  pub async fn run(
    &self,
    run_dir: &Utf8Path,
    builtins: &InternalCommands,
  ) -> Result<CommandResult, CommandError> {
    Ok(match self {
      Self::External { command, .. } => command.run(run_dir).await?,
      Self::Internal { name, .. } => builtins.get(name).ok_or(name)?(run_dir).await,
    })
  }
}

impl From<ExternalCommand> for Command {
  fn from(command: ExternalCommand) -> Self {
    Self::External { command, no_check: false }
  }
}

pub type InternalCommand = for<'p> fn(&'p Utf8Path) -> BoxFuture<'p, CommandResult>;
pub type InternalCommands = BTreeMap<String, InternalCommand>;

#[macro_export]
macro_rules! add_internal_command {
  ($map:expr, $name:expr, $func:ident) => {
    if $map.insert($name.to_string(), (|path| Box::pin($func(path)))).is_some() {
      panic!("Tried to insert multiple internal commands with the same name: {}", $name);
    }
  };
}

#[macro_export]
macro_rules! internal_commands {
  ($($name:expr => $func:ident),* $(,)?) => {{
    let mut map: $crate::flow::command::InternalCommands = Default::default();
    $(
      $crate::add_internal_command!(map, $name, $func);
    )*
    map
  }};
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalCommand {
  pub env: BTreeMap<String, String>,
  pub exec: Utf8PathBuf,
  pub args: Vec<String>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct CommandResult {
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
  pub status: ExitStatus,
}

impl ExternalCommand {
  pub async fn run(&self, run_dir: &Utf8Path) -> io::Result<CommandResult> {
    let output = process::Command::from(self).current_dir(run_dir).output().await?;
    Ok(CommandResult {
      stdout: output.stdout,
      stderr: output.stderr,
      status: exit_status_code(output.status),
    })
  }
}

/// A process terminated by a signal has no exit code;
/// it is reported using the shell convention of 128 + signal number.
fn exit_status_code(status: std::process::ExitStatus) -> ExitStatus {
  use std::os::unix::process::ExitStatusExt;
  status.code().or_else(|| status.signal().map(|signal| 128 + signal)).unwrap_or(-1)
}

impl From<&ExternalCommand> for process::Command {
  fn from(info: &ExternalCommand) -> Self {
    let mut cmd = Self::new(&info.exec);
    cmd.env_clear().envs(&info.env).args(&info.args).kill_on_drop(true);
    cmd
  }
}

#[derive(Debug, Error)]
pub enum CommandError {
  #[error("External command execution failed: {0}")]
  External(#[from] io::Error),
  #[error("Internal command '{0}' not found.")]
  InternalCommandNotFound(String),
}
impl From<&String> for CommandError {
  fn from(value: &String) -> Self {
    Self::InternalCommandNotFound(value.clone())
  }
}
