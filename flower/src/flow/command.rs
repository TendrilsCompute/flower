use camino::{Utf8Path, Utf8PathBuf};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio::{fs, io, process};

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

  /// The files this command reads its arguments from.
  pub fn arg_files(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    let args = match self {
      Self::External { command, .. } => command.args.as_slice(),
      Self::Internal { .. } => &[],
    };
    args.iter().filter_map(|arg| match arg {
      Arg::Lines(path) => Some(path),
      Arg::Literal(_) => None,
    })
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
  pub args: Vec<Arg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Arg {
  Literal(String),
  /// One argument per non-blank line of a file, read just before the command runs.
  /// A relative path is relative to the node's directory.
  Lines(Utf8PathBuf),
}
impl Arg {
  async fn resolve(&self, run_dir: &Utf8Path) -> Result<Vec<String>, CommandError> {
    match self {
      Self::Literal(arg) => Ok(vec![arg.clone()]),
      Self::Lines(path) => {
        let path = run_dir.join(path);
        let contents =
          fs::read_to_string(&path).await.map_err(|err| CommandError::ArgFile(path, err))?;
        Ok(contents.lines().filter(|line| !line.trim().is_empty()).map(str::to_string).collect())
      }
    }
  }
}

impl From<String> for Arg {
  fn from(arg: String) -> Self {
    Self::Literal(arg)
  }
}
impl From<&str> for Arg {
  fn from(arg: &str) -> Self {
    Self::Literal(arg.to_string())
  }
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct CommandResult {
  #[serde(serialize_with = "as_bytes")]
  pub stdout: Vec<u8>,
  #[serde(serialize_with = "as_bytes")]
  pub stderr: Vec<u8>,
  pub status: ExitStatus,
}

/// If this function isn't used, an empty `Vec<u8>` isn't serialized as bytes.
fn as_bytes<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
  serializer.serialize_bytes(bytes)
}

impl ExternalCommand {
  pub async fn run(&self, run_dir: &Utf8Path) -> Result<CommandResult, CommandError> {
    let mut args = Vec::new();
    for arg in &self.args {
      args.extend(arg.resolve(run_dir).await?);
    }
    let output = process::Command::new(&self.exec)
      .env_clear()
      .envs(&self.env)
      .args(args)
      .current_dir(run_dir)
      .kill_on_drop(true)
      .output()
      .await?;
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

#[derive(Debug, Error)]
pub enum CommandError {
  #[error("External command execution failed: {0}")]
  External(#[from] io::Error),
  #[error("Failed to read arguments from {0}: {1}")]
  ArgFile(Utf8PathBuf, io::Error),
  #[error("Internal command '{0}' not found.")]
  InternalCommandNotFound(String),
}
impl From<&String> for CommandError {
  fn from(value: &String) -> Self {
    Self::InternalCommandNotFound(value.clone())
  }
}
