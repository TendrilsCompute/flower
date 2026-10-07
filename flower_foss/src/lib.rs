pub mod flow;
pub mod project;
pub mod sv;
pub mod tool;

use std::{collections::BTreeSet, path::PathBuf};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use walkdir::WalkDir;
use which::{Error as WhichError, which};

pub fn get_executable(name: &str) -> Result<Utf8PathBuf, FlowBuildError> {
  use FlowBuildError::*;
  let path = which(name).map_err(|e| ExecutableFindError(name.to_string(), e))?;
  Utf8PathBuf::from_path_buf(path).map_err(NonUtf8Path)
}

#[derive(Debug, Error)]
pub enum FlowBuildError {
  #[error("Failed to find executable '{0}': {1}")]
  ExecutableFindError(String, WhichError),
  #[error("Non utf8 path '{0}'")]
  NonUtf8Path(PathBuf),
}
pub type FlowBuildResult<T> = Result<T, FlowBuildError>;

fn files_with_extension(dir: impl AsRef<Utf8Path>, ext: &str) -> impl Iterator<Item = Utf8PathBuf> {
  WalkDir::new(dir.as_ref())
    .into_iter()
    .filter_map(Result::ok)
    .filter(|entry| entry.file_type().is_file())
    .filter(|entry| entry.path().extension().is_some_and(|entry_ext| *entry_ext == *ext))
    .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.into_path()).ok())
}

pub fn generate_path<'a>(execs: impl IntoIterator<Item = &'a str>) -> FlowBuildResult<String> {
  let mut paths = BTreeSet::new();
  for exec in execs {
    let mut path = get_executable(exec)?;
    path.pop();
    paths.insert(path.into_string());
  }
  Ok(paths.into_iter().collect::<Vec<_>>().join(":"))
}
