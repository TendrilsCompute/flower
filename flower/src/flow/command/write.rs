//! The workings of [`Command::Write`](super::Command::Write):
//! writing a file from literals and regex extractions of other files.

use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::fs;

use super::CommandResult;

/// Part of the text a [`Command::Write`](super::Command::Write) writes.
/// A relative path is relative to the node's directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TextPart {
  Literal(String),
  /// The single match in a file; it's an error if there are none or several.
  Match {
    path: Utf8PathBuf,
    extract: Extract,
  },
  /// Every match in a file, each followed by a newline.
  Matches {
    path: Utf8PathBuf,
    extract: Extract,
  },
}
impl TextPart {
  /// The single non-blank line of a file, trimmed.
  pub fn file(path: impl Into<Utf8PathBuf>) -> Self {
    Self::Match { path: path.into(), extract: Extract::whole(r"\S(?:.*\S)?") }
  }

  pub(super) fn path(&self) -> Option<&Utf8PathBuf> {
    match self {
      Self::Literal(_) => None,
      Self::Match { path, .. } | Self::Matches { path, .. } => Some(path),
    }
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

  /// Extracts whole matches of a pattern known to be valid.
  fn whole(pattern: &str) -> Self {
    Self { pattern: pattern.to_string(), template: "$0".to_string() }
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
      TextPart::Match { path, extract } | TextPart::Matches { path, extract } => {
        let contents = fs::read_to_string(run_dir.join(path))
          .await
          .map_err(|e| format!("Failed to read {path}: {e}"))?;
        let matches = extract.all(&contents).map_err(|e| format!("Invalid regex: {e}"))?;
        if let TextPart::Match { .. } = part {
          let [single] = matches.as_slice() else {
            return Err(format!("Expected a single match in {path}, found {}", matches.len()));
          };
          text.push_str(single);
        } else {
          for single in matches {
            text.push_str(&single);
            text.push('\n');
          }
        }
      }
    }
  }
  fs::write(run_dir.join(path), text).await.map_err(|e| format!("Failed to write {path}: {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn all(extract: &Extract, haystack: &str) -> Vec<String> {
    extract.all(haystack).unwrap()
  }

  #[test]
  fn file_is_the_trimmed_non_blank_line() {
    let TextPart::Match { extract, .. } = TextPart::file("f") else { unreachable!() };
    assert_eq!(all(&extract, "\n  /a/b c  \n\n"), ["/a/b c"]);
    assert_eq!(all(&extract, "a\nb\n").len(), 2);
    assert!(all(&extract, " \n").is_empty());
  }

  #[test]
  fn captures_are_expanded() {
    let extract = Extract::new(r"/lib([^/\s]+)\.so\s*$", "$1").unwrap();
    assert_eq!(all(&extract, "/x/libcocotbvpi_verilator.so\n"), ["cocotbvpi_verilator"]);
  }

  #[test]
  fn only_matches_are_extracted() {
    let extract = Extract::new(r"(?m)^Slack: (\S+)$", "--slack=$1").unwrap();
    let log = "Starting\nSlack: 0.12\nnoise Slack: 9\nSlack: -0.3\nDone\n";
    assert_eq!(all(&extract, log), ["--slack=0.12", "--slack=-0.3"]);
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

    let parts = vec![
      "-l".into(),
      TextPart::Match {
        path: "lib".into(),
        extract: Extract::new(r"/lib(\w+)\.so", "$1").unwrap(),
      },
      "\n".into(),
      TextPart::Matches { path: "log".into(), extract: Extract::new(r"a=(\d)", "-D$1").unwrap() },
    ];
    write_text(&dir, "out".into(), &parts).await.unwrap();
    assert_eq!(fs::read_to_string(dir.join("out")).await.unwrap(), "-lfoo\n-D1\n-D2\n");

    let ambiguous = [TextPart::Match { path: "log".into(), extract: Extract::whole("a") }];
    assert!(write_text(&dir, "out".into(), &ambiguous).await.is_err());

    fs::remove_dir_all(&dir).await.unwrap();
  }
}
