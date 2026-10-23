use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio::{fs, io, process};

mod write;
pub use write::{Extract, FromFile, MatchCount, TextPart};

pub type ExitStatus = i32;
pub type Env = BTreeMap<String, EnvValue>;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum Command {
  External {
    #[serde(flatten)]
    command: ExternalCommand,
    no_check: bool,
  },
  /// Writes a file from the parts, concatenated.
  /// A relative path is relative to the node's directory.
  Write { path: Utf8PathBuf, parts: Vec<TextPart>, no_check: bool },
}
impl Command {
  pub fn write(path: impl Into<Utf8PathBuf>, parts: Vec<TextPart>) -> Self {
    Self::Write { path: path.into(), parts, no_check: false }
  }

  pub fn no_check(&self) -> bool {
    match self {
      Self::External { no_check, .. } | Self::Write { no_check, .. } => *no_check,
    }
  }

  /// The files this command reads, other than its executable:
  /// the files its arguments and environment are read from, or the files it writes from.
  pub fn input_files(&self) -> Vec<&Utf8PathBuf> {
    match self {
      Self::External { command, .. } => {
        let arg_files = command.args.iter().filter_map(|arg| match arg {
          Arg::Lines(path) => Some(path),
          Arg::Literal(_) => None,
        });
        let env_files = command.env.values().filter_map(|value| match value {
          EnvValue::File(path) => Some(path),
          EnvValue::Literal(_) => None,
        });
        arg_files.chain(env_files).collect()
      }
      Self::Write { parts, .. } => parts.iter().filter_map(TextPart::path).collect(),
    }
  }

  /// The files this command writes, other than through what it runs.
  pub fn output_files(&self) -> Vec<&Utf8PathBuf> {
    match self {
      Self::External { command, .. } => command.stdout.iter().collect(),
      Self::Write { path, .. } => vec![path],
    }
  }

  /// The executable an external command runs.
  pub fn exec(&self) -> Option<&Utf8PathBuf> {
    match self {
      Self::External { command, .. } => Some(&command.exec),
      Self::Write { .. } => None,
    }
  }

  pub async fn run(&self, run_dir: &Utf8Path) -> Result<CommandResult, CommandError> {
    Ok(match self {
      Self::External { command, .. } => command.run(run_dir).await?,
      Self::Write { path, parts, .. } => write::run(run_dir, path, parts).await,
    })
  }
}

impl From<ExternalCommand> for Command {
  fn from(command: ExternalCommand) -> Self {
    Self::External { command, no_check: false }
  }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalCommand {
  pub env: Env,
  /// A relative path is relative to the node's directory.
  pub exec: Utf8PathBuf,
  pub args: Vec<Arg>,
  /// A file stdout is also written to. A relative path is relative to the node's directory.
  pub stdout: Option<Utf8PathBuf>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EnvValue {
  Literal(String),
  /// The contents of a file without leading or trailing whitespace, read just before the command
  /// runs. A relative path is relative to the node's directory.
  File(Utf8PathBuf),
}
impl EnvValue {
  async fn resolve(&self, run_dir: &Utf8Path) -> Result<String, CommandError> {
    match self {
      Self::Literal(value) => Ok(value.clone()),
      Self::File(path) => {
        let path = run_dir.join(path);
        let contents =
          fs::read_to_string(&path).await.map_err(|err| CommandError::EnvFile(path, err))?;
        Ok(contents.trim().to_string())
      }
    }
  }
}

impl From<String> for EnvValue {
  fn from(value: String) -> Self {
    Self::Literal(value)
  }
}
impl From<&str> for EnvValue {
  fn from(value: &str) -> Self {
    Self::Literal(value.to_string())
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
    let mut env = Vec::new();
    for (name, value) in &self.env {
      env.push((name, value.resolve(run_dir).await?));
    }
    // How a relative program is found with `current_dir` differs between platforms.
    let output = process::Command::new(run_dir.join(&self.exec))
      .env_clear()
      .envs(env)
      .args(args)
      .current_dir(run_dir)
      .kill_on_drop(true)
      .output()
      .await?;
    if let Some(path) = &self.stdout {
      let path = run_dir.join(path);
      fs::write(&path, &output.stdout).await.map_err(|err| CommandError::StdoutFile(path, err))?;
    }
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
  #[error("Failed to read an environment variable from {0}: {1}")]
  EnvFile(Utf8PathBuf, io::Error),
  #[error("Failed to write stdout to {0}: {1}")]
  StdoutFile(Utf8PathBuf, io::Error),
}
