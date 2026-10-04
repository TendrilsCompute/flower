use std::io::{self, Write};

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};

use flower::hash::PathHasher;

#[derive(Parser)]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Output (as raw bytes) the hash of a path
  Hash { path: Utf8PathBuf },
}

fn main() {
  let Command::Hash { path } = Cli::parse().command;

  let hash = PathHasher::default().hash(&path).unwrap();

  io::stdout().lock().write_all(&hash).unwrap();
}
