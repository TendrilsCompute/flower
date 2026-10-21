use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use merkle_hash::{MerkleTree, error::IndexingError};
use thiserror::Error;
use tokio::task::{self, JoinError};

pub type PathHash = Vec<u8>;

#[derive(Default)]
pub struct PathHasher {
  cache: HashMap<Utf8PathBuf, PathHash>,
}
impl PathHasher {
  pub async fn hash(&mut self, path: &Utf8Path) -> Result<PathHash, HashingError> {
    if let Some(hash) = self.cache.get(path) {
      return Ok(hash.clone());
    }
    // Building the tree blocks (merkle_hash parallelises it with rayon),
    // so it is kept off the async runtime's thread.
    let root = path.to_owned();
    let tree = task::spawn_blocking(move || MerkleTree::builder(root).build()).await??;
    // If a subdirectory has previously been hashed,
    // we are sadly not reusing it when hashing the parent directory.
    let hash = tree.root.item.hash.clone();
    for item in tree.into_iter() {
      self.cache.insert(item.path.absolute, item.hash);
    }
    Ok(hash)
  }

  pub fn remove(&mut self, path: &Utf8Path) {
    self.cache.retain(|cache_path, _| !cache_path.starts_with(path))
  }
}

#[derive(Debug, Error)]
pub enum HashingError {
  #[error(transparent)]
  Indexing(#[from] IndexingError),
  #[error("Hashing task didn't complete: {0}")]
  PanicOrAbort(#[from] JoinError),
}
