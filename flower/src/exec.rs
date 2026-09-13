use serde::{Deserialize, Serialize};
use std::{
  collections::BTreeMap,
  ffi::OsString,
  path::{Path, PathBuf},
};
use tokio::io::{self};
use tokio::process::Command;

pub type ExitStatus = i32;

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ExecRunInfo {
  pub env: BTreeMap<OsString, OsString>,
  pub exec: PathBuf,
  pub no_check: bool,
  pub args: Vec<OsString>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecRunResult {
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
  pub status: ExitStatus,
}

impl ExecRunInfo {
  pub async fn run(&self, run_dir: &Path) -> io::Result<ExecRunResult> {
    let output = Command::from(self).current_dir(run_dir).output().await?;
    Ok(ExecRunResult {
      stdout: output.stdout,
      stderr: output.stderr,
      status: output.status.code().unwrap_or(0),
    })
  }
}

impl From<&ExecRunInfo> for Command {
  fn from(info: &ExecRunInfo) -> Self {
    let mut cmd = Self::new(&info.exec);
    cmd.env_clear().envs(&info.env).args(&info.args);
    cmd
  }
}
