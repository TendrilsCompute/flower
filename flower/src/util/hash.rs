use std::{
  collections::{HashMap, HashSet},
  fs::{self, File},
  io,
  os::unix::fs::MetadataExt,
  sync::{Arc, Mutex},
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use tokio::task::{self, JoinError};

pub type PathHash = Vec<u8>;
type Cache = HashMap<Utf8PathBuf, PathHash>;

/// Hashes paths, caching the hashes so far, which concurrent hashes share.
#[derive(Clone, Default)]
pub struct PathHasher {
  cache: Arc<Mutex<Cache>>,
}
impl PathHasher {
  pub async fn hash(&self, path: &Utf8Path) -> Result<PathHash, HashingError> {
    // Hashing blocks, so it is kept off the async runtime's thread.
    let (root, cache) = (path.to_owned(), self.cache.clone());
    task::spawn_blocking(move || hash_path(&root, &mut HashSet::new(), &cache)).await?
  }
}

/// Hashes the contents of `path`, following links, and the names in it, caching the hash of each
/// path within it. A dangling link is hashed by its target.
/// `ancestors` are the directories `path` is in, by device and inode, which a link mustn't lead to.
fn hash_path(
  path: &Utf8Path,
  ancestors: &mut HashSet<(u64, u64)>,
  cache: &Mutex<Cache>,
) -> Result<PathHash, HashingError> {
  if let Some(hash) = cache.lock().expect("The cache isn't poisoned.").get(path) {
    return Ok(hash.clone());
  }
  let io_err = |err| HashingError::Io(path.to_owned(), err);
  let mut hasher = blake3::Hasher::new();
  match fs::metadata(path) {
    Err(err) if err.kind() == io::ErrorKind::NotFound && path.is_symlink() => {
      let target = fs::read_link(path).map_err(io_err)?;
      hasher.update(b"link").update(target.as_os_str().as_encoded_bytes());
    }
    Err(err) => return Err(io_err(err)),
    Ok(metadata) if metadata.is_dir() => {
      let id = (metadata.dev(), metadata.ino());
      if !ancestors.insert(id) {
        return Err(HashingError::LinkLoop(path.to_owned()));
      }
      let mut names = Vec::new();
      for entry in path.read_dir_utf8().map_err(io_err)? {
        names.push(entry.map_err(io_err)?.file_name().to_owned());
      }
      names.sort();
      hasher.update(b"dir");
      for name in names {
        hasher.update(&[0]);
        let hash = hash_path(&path.join(&name), ancestors, cache)?;
        hasher.update(name.as_bytes()).update(&hash);
      }
      ancestors.remove(&id);
    }
    Ok(metadata) => {
      let kind: &[u8] = if metadata.mode() & 0o111 != 0 { b"exec" } else { b"file" };
      hasher.update(kind).update_reader(File::open(path).map_err(io_err)?).map_err(io_err)?;
    }
  }
  let hash = hasher.finalize().as_bytes().to_vec();
  cache.lock().expect("The cache isn't poisoned.").insert(path.to_owned(), hash.clone());
  Ok(hash)
}

#[derive(Debug, Error)]
pub enum HashingError {
  #[error("Failed to hash {0}: {1}")]
  Io(Utf8PathBuf, io::Error),
  #[error("{0} links to a directory it's in")]
  LinkLoop(Utf8PathBuf),
  #[error("Hashing task didn't complete: {0}")]
  PanicOrAbort(#[from] JoinError),
}
