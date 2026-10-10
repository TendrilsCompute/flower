use std::path::PathBuf;

use camino::Utf8PathBuf;
use thiserror::Error;
use tokio::io;

#[derive(Debug, Error)]
pub enum ConstructionError {
  #[error("Canonicalization of {0} failed: {1}")]
  Canonicalization(Utf8PathBuf, io::Error),
  #[error("Io error involving {0}: {1}")]
  IoError(Utf8PathBuf, io::Error),
  #[error("Non utf8 path '{0}'")]
  NonUtf8Path(PathBuf),
}
pub type ConstructionResult<T> = Result<T, ConstructionError>;

impl From<PathBuf> for ConstructionError {
  fn from(path: PathBuf) -> Self {
    Self::NonUtf8Path(path)
  }
}
