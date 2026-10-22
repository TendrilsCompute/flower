//! The workings of [`Command::Write`](super::Command::Write):
//! writing a file from literals and regex extractions of other files.

use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::fs;

use super::CommandResult;

/// Part of the text a [`Command::Write`](super::Command::Write) writes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TextPart {
  Literal(String),
  FromFile(FromFile),
}
impl TextPart {
  pub(super) fn path(&self) -> Option<&Utf8PathBuf> {
    match self {
      Self::Literal(_) => None,
      Self::FromFile(from_file) => Some(&from_file.path),
    }
  }
}

/// The matches in a file, expanded and concatenated.
/// A relative path is relative to the node's directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FromFile {
  pub path: Utf8PathBuf,
  pub extract: Extract,
  pub count: MatchCount,
  /// Drop repeated expansions, keeping the first of each. `count` applies afterwards.
  pub distinct: bool,
}
impl FromFile {
  pub fn new(path: impl Into<Utf8PathBuf>, extract: Extract) -> Self {
    Self { path: path.into(), extract, count: MatchCount::default(), distinct: false }
  }

  /// The contents of a file, without leading or trailing whitespace.
  /// It's an error if the file is blank.
  pub fn contents(path: impl Into<Utf8PathBuf>) -> Self {
    Self::new(path, Extract::valid(r"(?s)\S(?:.*\S)?", "$0")).count(MatchCount::ExactlyOne)
  }

  pub fn count(self, count: MatchCount) -> Self {
    Self { count, ..self }
  }

  pub fn distinct(self) -> Self {
    Self { distinct: true, ..self }
  }
}

/// How many matches a [`FromFile`] expects; any other number is an error.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum MatchCount {
  ZeroOrMore,
  ExactlyOne,
  #[default]
  OneOrMore,
}
impl MatchCount {
  fn allows(self, count: usize) -> bool {
    match self {
      Self::ZeroOrMore => true,
      Self::ExactlyOne => count == 1,
      Self::OneOrMore => count >= 1,
    }
  }
}

impl From<FromFile> for TextPart {
  fn from(from_file: FromFile) -> Self {
    Self::FromFile(from_file)
  }
}
impl From<String> for TextPart {
  fn from(literal: String) -> Self {
    Self::Literal(literal)
  }
}
impl From<&str> for TextPart {
  fn from(literal: &str) -> Self {
    Self::Literal(literal.to_string())
  }
}

/// A regex and the template each of its matches expands to (`$0`, `$1`, `$name`),
/// as in [`regex::Captures::expand`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Extract {
  pub pattern: String,
  pub template: String,
}
impl Extract {
  pub fn new(
    pattern: impl Into<String>,
    template: impl Into<String>,
  ) -> Result<Self, regex::Error> {
    let pattern = pattern.into();
    Regex::new(&pattern)?;
    Ok(Self { pattern, template: template.into() })
  }

  /// For patterns written in this module, which the tests check are valid.
  fn valid(pattern: &str, template: &str) -> Self {
    Self { pattern: pattern.to_string(), template: template.to_string() }
  }

  fn all(&self, haystack: &str) -> Result<Vec<String>, regex::Error> {
    let regex = Regex::new(&self.pattern)?;
    Ok(
      regex
        .captures_iter(haystack)
        .map(|captures| {
          let mut expanded = String::new();
          captures.expand(&self.template, &mut expanded);
          expanded
        })
        .collect(),
    )
  }
}

pub(super) async fn run(run_dir: &Utf8Path, path: &Utf8Path, parts: &[TextPart]) -> CommandResult {
  match write_text(run_dir, path, parts).await {
    Ok(()) => CommandResult::default(),
    Err(message) => CommandResult { stderr: message.into(), stdout: Default::default(), status: 1 },
  }
}

async fn write_text(run_dir: &Utf8Path, path: &Utf8Path, parts: &[TextPart]) -> Result<(), String> {
  let mut text = String::new();
  for part in parts {
    match part {
      TextPart::Literal(literal) => text.push_str(literal),
      TextPart::FromFile(FromFile { path, extract, count, distinct }) => {
        let contents = fs::read_to_string(run_dir.join(path))
          .await
          .map_err(|e| format!("Failed to read {path}: {e}"))?;
        let mut matches = extract.all(&contents).map_err(|e| format!("Invalid regex: {e}"))?;
        if *distinct {
          let mut seen = HashSet::new();
          matches.retain(|expanded| seen.insert(expanded.clone()));
        }
        if !count.allows(matches.len()) {
          return Err(format!("Expected {count:?} matches in {path}, found {}", matches.len()));
        }
        text.extend(matches);
      }
    }
  }
  fs::write(run_dir.join(path), text).await.map_err(|e| format!("Failed to write {path}: {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn all(from_file: &FromFile, haystack: &str) -> Vec<String> {
    from_file.extract.all(haystack).unwrap()
  }

  #[test]
  fn helper_patterns_are_valid() {
    let contents = FromFile::contents("f");
    Extract::new(contents.extract.pattern, contents.extract.template).unwrap();
  }

  #[test]
  fn contents_is_trimmed() {
    let file = FromFile::contents("f");
    assert_eq!(all(&file, "\n  /a/b c  \n\n"), ["/a/b c"]);
    assert_eq!(all(&file, " a\n\nb \n"), ["a\n\nb"]);
    assert!(all(&file, " \n").is_empty());
  }

  #[test]
  fn captures_are_expanded() {
    let extract = Extract::new(r"/lib([^/\s]+)\.so\s*$", "$1").unwrap();
    let from_file = FromFile::new("f", extract);
    assert_eq!(all(&from_file, "/x/libcocotbvpi_verilator.so\n"), ["cocotbvpi_verilator"]);
  }

  #[test]
  fn only_matches_are_extracted() {
    let extract = Extract::new(r"(?m)^Slack: (\S+)$", "--slack=$1").unwrap();
    let log = "Starting\nSlack: 0.12\nnoise Slack: 9\nSlack: -0.3\nDone\n";
    assert_eq!(all(&FromFile::new("f", extract), log), ["--slack=0.12", "--slack=-0.3"]);
  }

  #[test]
  fn invalid_regex_is_rejected() {
    assert!(Extract::new("(", "$0").is_err());
  }

  #[tokio::test]
  async fn write_concatenates_parts() {
    let dir = Utf8PathBuf::from_path_buf(std::env::temp_dir())
      .unwrap()
      .join(format!("flower-write-test-{}", std::process::id()));
    fs::create_dir_all(&dir).await.unwrap();
    fs::write(dir.join("lib"), "/x/libfoo.so\n").await.unwrap();
    fs::write(dir.join("log"), "a=1\nnoise\na=2\n").await.unwrap();
    let write = |parts: Vec<TextPart>| {
      let dir = dir.clone();
      async move {
        write_text(&dir, "out".into(), &parts).await?;
        Ok::<_, String>(fs::read_to_string(dir.join("out")).await.unwrap())
      }
    };

    let lib = FromFile::new("lib", Extract::new(r"/lib(\w+)\.so", "$1").unwrap());
    let defines = FromFile::new("log", Extract::new(r"a=(\d)", "-D$1\n").unwrap());
    let written = write(vec!["-l".into(), lib.into(), "\n".into(), defines.into()]).await;
    assert_eq!(written.unwrap(), "-lfoo\n-D1\n-D2\n");

    let a = Extract::new("a", "$0").unwrap();
    let miscounted = FromFile::new("log", a).count(MatchCount::ExactlyOne);
    assert!(write(vec![miscounted.into()]).await.is_err());

    let missing = FromFile::new("log", Extract::new("b", "$0").unwrap());
    assert!(write(vec![missing.into()]).await.is_err());

    fs::write(dir.join("includes"), "/a/x.svh\n/b/y.svh\n/a/z.svh\n").await.unwrap();
    let dirs = FromFile::new("includes", Extract::new(r"(?m)^(.*)/[^/]*$", "$1\n").unwrap());
    assert_eq!(write(vec![dirs.distinct().into()]).await.unwrap(), "/a\n/b\n");

    fs::remove_dir_all(&dir).await.unwrap();
  }
}
