use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio::{fs, io, process};

mod write;
pub use write::{Extract, FromFile, MatchCount, TextPart};

pub type ExitStatus = i32;
pub type Env<V = Value> = BTreeMap<String, V>;

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
        command.args.iter().chain(command.env.values()).filter_map(Value::path).collect()
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
      Self::External { command, .. } => command.resolve(run_dir).await?.run(run_dir).await?,
      Self::Write { path, parts, .. } => write::run(run_dir, path, parts).await,
    })
  }
}

impl From<ExternalCommand> for Command {
  fn from(command: ExternalCommand) -> Self {
    Self::External { command, no_check: false }
  }
}

/// A program to run, with [`Value`]s for its arguments and environment,
/// or once they're resolved, `String`s.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalCommand<V = Value> {
  pub env: Env<V>,
  pub exec: Utf8PathBuf,
  pub args: Vec<V>,
  /// A file stdout is also written to.
  pub stdout: Option<Utf8PathBuf>,
}
impl<V> Default for ExternalCommand<V> {
  fn default() -> Self {
    Self { env: Env::new(), exec: Utf8PathBuf::new(), args: Vec::new(), stdout: None }
  }
}

/// An argument or environment variable's value: a literal, or a file read just before the
/// command runs.
///
/// When taking values from a file:
/// - An environment variable takes the whole content of the file as a value.
/// - An argument is added for each line of the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Value {
  Literal(String),
  FromFile(Utf8PathBuf),
}
impl Value {
  fn path(&self) -> Option<&Utf8PathBuf> {
    match self {
      Self::Literal(_) => None,
      Self::FromFile(path) => Some(path),
    }
  }

  /// The file's contents, failing with `err`.
  async fn read(
    path: &Utf8Path,
    run_dir: &Utf8Path,
    err: fn(Utf8PathBuf, io::Error) -> CommandError,
  ) -> Result<String, CommandError> {
    let path = run_dir.join(path);
    fs::read_to_string(&path).await.map_err(|e| err(path, e))
  }

  async fn resolve_args(&self, run_dir: &Utf8Path) -> Result<Vec<String>, CommandError> {
    match self {
      Self::Literal(arg) => Ok(vec![arg.clone()]),
      Self::FromFile(path) => {
        let contents = Self::read(path, run_dir, CommandError::ArgFile).await?;
        Ok(contents.lines().filter(|line| !line.trim().is_empty()).map(str::to_string).collect())
      }
    }
  }

  async fn resolve_env(&self, run_dir: &Utf8Path) -> Result<String, CommandError> {
    match self {
      Self::Literal(value) => Ok(value.clone()),
      Self::FromFile(path) => {
        let contents = Self::read(path, run_dir, CommandError::EnvFile).await?;
        Ok(contents.trim().to_string())
      }
    }
  }
}

impl From<String> for Value {
  fn from(value: String) -> Self {
    Self::Literal(value)
  }
}
impl From<&str> for Value {
  fn from(value: &str) -> Self {
    Self::Literal(value.to_string())
  }
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct CommandResult {
  /// An external command as it ran, with its values resolved.
  pub command: Option<ExternalCommand<String>>,
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
  /// The command as it runs in `run_dir`, with its values read.
  pub async fn resolve(&self, run_dir: &Utf8Path) -> Result<ExternalCommand<String>, CommandError> {
    let mut args = Vec::new();
    for arg in &self.args {
      args.extend(arg.resolve_args(run_dir).await?);
    }
    let mut env = Env::new();
    for (name, value) in &self.env {
      env.insert(name.clone(), value.resolve_env(run_dir).await?);
    }
    Ok(ExternalCommand { env, exec: self.exec.clone(), args, stdout: self.stdout.clone() })
  }
}

impl ExternalCommand<String> {
  pub async fn run(self, run_dir: &Utf8Path) -> Result<CommandResult, CommandError> {
    // How a relative program is found with `current_dir` differs between platforms.
    let output = process::Command::new(run_dir.join(&self.exec))
      .env_clear()
      .envs(&self.env)
      .args(&self.args)
      .current_dir(run_dir)
      .kill_on_drop(true)
      .output()
      .await
      .map_err(|err| CommandError::External(self.exec.clone(), err))?;
    if let Some(path) = &self.stdout {
      let path = run_dir.join(path);
      fs::write(&path, &output.stdout).await.map_err(|err| CommandError::StdoutFile(path, err))?;
    }
    Ok(CommandResult {
      command: Some(self),
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
  #[error("{0}: {1}")]
  External(Utf8PathBuf, io::Error),
  #[error("Failed to read arguments from {0}: {1}")]
  ArgFile(Utf8PathBuf, io::Error),
  #[error("Failed to read an environment variable from {0}: {1}")]
  EnvFile(Utf8PathBuf, io::Error),
  #[error("Failed to write stdout to {0}: {1}")]
  StdoutFile(Utf8PathBuf, io::Error),
}
