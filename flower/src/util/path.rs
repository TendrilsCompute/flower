use camino::{Utf8Path, Utf8PathBuf};
use std::path::PathBuf;
use tokio::{fs, io};

/// The absolute path of `path`, with links resolved.
pub async fn canonicalize(path: &Utf8Path) -> io::Result<Utf8PathBuf> {
  utf8(fs::canonicalize(path).await?)
}

/// `path` as UTF-8, or an IO error if it isn't.
pub fn utf8(path: PathBuf) -> io::Result<Utf8PathBuf> {
  Utf8PathBuf::try_from(path).map_err(|err| io::Error::new(io::ErrorKind::InvalidFilename, err))
}
