use std::env;

use camino::{Utf8Path, Utf8PathBuf};
use tokio::fs;

use crate::construct::{ConstructionError, ConstructionResult};

pub async fn canonicalize(path: &Utf8Path) -> ConstructionResult<Utf8PathBuf> {
  let canonicalized = fs::canonicalize(path)
    .await
    .map_err(|e| ConstructionError::Canonicalization(Utf8PathBuf::from(path), e))?;
  Ok(Utf8PathBuf::from_path_buf(canonicalized)?)
}

pub async fn canonicalize_on(path: &Utf8Path, root: &Utf8Path) -> ConstructionResult<Utf8PathBuf> {
  let absolute = root.join(path);
  canonicalize(&absolute).await
}

pub fn current_dir() -> ConstructionResult<Utf8PathBuf> {
  let dir =
    env::current_dir().map_err(|e| ConstructionError::IoError(Utf8PathBuf::from("."), e))?;
  Ok(Utf8PathBuf::from_path_buf(dir)?)
}

pub async fn find_file_in_parents(
  start_path: &Utf8Path,
  file_name: &str,
) -> ConstructionResult<Option<Utf8PathBuf>> {
  let mut path = canonicalize(start_path).await?;
  if path.is_file() {
    path.pop();
  }
  loop {
    path.push(file_name);
    if fs::try_exists(&path).await.map_err(|e| ConstructionError::IoError(path.clone(), e))? {
      return Ok(Some(path));
    }
    path.pop();
    if !path.pop() {
      break;
    }
  }
  Ok(None)
}
