pub mod path;

use std::{collections::BTreeSet, path::PathBuf};

use camino::Utf8PathBuf;
use thiserror::Error;
use tokio::io;
use which::{Error as WhichError, which};

pub fn get_executable(name: &str) -> ConstructionResult<Utf8PathBuf> {
  use ConstructionError::*;
  let path = which(name).map_err(|e| ExecutableFindError(name.to_string(), e))?;
  Utf8PathBuf::from_path_buf(path).map_err(NonUtf8Path)
}

pub fn generate_path<'a>(execs: impl IntoIterator<Item = &'a str>) -> ConstructionResult<String> {
  let mut paths = BTreeSet::new();
  for exec in execs {
    let mut path = get_executable(exec)?;
    path.pop();
    paths.insert(path.into_string());
  }
  Ok(paths.into_iter().collect::<Vec<_>>().join(":"))
}

#[derive(Debug, Error)]
pub enum ConstructionError {
  #[error("Canonicalization of {0} failed: {1}")]
  Canonicalization(Utf8PathBuf, io::Error),
  #[error("Io error involving {0}: {1}")]
  IoError(Utf8PathBuf, io::Error),
  #[error("Non utf8 path '{0}'")]
  NonUtf8Path(PathBuf),
  #[error("Failed to find executable '{0}': {1}")]
  ExecutableFindError(String, WhichError),
}
pub type ConstructionResult<T> = Result<T, ConstructionError>;

impl From<PathBuf> for ConstructionError {
  fn from(path: PathBuf) -> Self {
    Self::NonUtf8Path(path)
  }
}
