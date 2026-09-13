use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use merkle_hash::{MerkleTree, error::IndexingError};

pub type PathHash = Vec<u8>;
pub type HashingError = IndexingError;

#[derive(Default)]
pub struct PathHasher {
  cache: HashMap<Utf8PathBuf, PathHash>,
}
impl PathHasher {
  pub fn hash(&mut self, path: &Utf8Path) -> Result<PathHash, IndexingError> {
    if let Some(hash) = self.cache.get(path) {
      return Ok(hash.clone())
    }
    // If a subdirectory has previously been hashed,
    // we are sadly not reusing it when hashing the parent directory.
    let tree = MerkleTree::builder(path).build()?;
    let hash = tree.root.item.hash.clone();
    for item in tree.into_iter() {
      self.cache.insert(item.path.absolute, item.hash);
    }
    Ok(hash)
  }

  pub fn remove(&mut self, path: &Utf8Path) {
    self.cache.retain(|cache_path, _ | !cache_path.starts_with(path))
  }
}
