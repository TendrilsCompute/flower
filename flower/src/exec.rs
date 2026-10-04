use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::io::{self};
use tokio::process::Command;

pub type ExitStatus = i32;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecRunInfo {
  pub env: BTreeMap<String, String>,
  pub exec: Utf8PathBuf,
  pub no_check: bool,
  pub args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecRunResult {
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
  pub status: ExitStatus,
}

impl ExecRunInfo {
  pub async fn run(&self, run_dir: &Utf8Path) -> io::Result<ExecRunResult> {
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
