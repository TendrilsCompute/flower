pub mod flow;
pub mod util;

use camino::Utf8Path;
use tokio::{fs, io};

pub async fn clean_dir(path: &Utf8Path) -> io::Result<()> {
  match fs::remove_dir_all(path).await {
    Ok(_) => {}
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
    Err(e) => return Err(e),
  }
  fs::create_dir_all(path).await
}
