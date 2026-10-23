//! What a path in a node's definition refers to.

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use thiserror::Error;

/// A path in a node's definition: `…` or `./…` in the node's own directory, `../<node>/…` in
/// another node's, which are its siblings, or `/…` outside the flow.
/// Otherwise, only normal components are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodePath<'a> {
  /// `<path>` or `./<path>`: in the node's own directory.
  Local(&'a Utf8Path),
  /// `../<node>/<path>`: in the directory of another node of the flow.
  OtherNode { node: &'a str, path: &'a Utf8Path },
  /// `/<path>`: outside the flow.
  External(&'a Utf8Path),
}

impl<'a> NodePath<'a> {
  pub fn parse(path: &'a Utf8Path) -> Result<Self, InvalidPath> {
    let invalid = || InvalidPath(path.to_owned());
    let mut components = path.components();
    let parsed = match components.next() {
      Some(Utf8Component::RootDir) => Self::External(path),
      Some(Utf8Component::CurDir) => Self::Local(components.as_path()),
      Some(Utf8Component::Normal(_)) => {
        components = path.components();
        Self::Local(path)
      }
      Some(Utf8Component::ParentDir) => match components.next() {
        Some(Utf8Component::Normal(node)) => Self::OtherNode { node, path: components.as_path() },
        _ => return Err(invalid()),
      },
      Some(Utf8Component::Prefix(_)) | None => return Err(invalid()),
    };
    if components.all(|component| matches!(component, Utf8Component::Normal(_))) {
      Ok(parsed)
    } else {
      Err(invalid())
    }
  }

  pub fn resolve(&self, node_dir: &Utf8Path) -> Utf8PathBuf {
    match *self {
      Self::Local(path) => join(node_dir, path),
      Self::OtherNode { node, path } => {
        let flow_dir = node_dir.parent().unwrap_or(Utf8Path::new(""));
        join(&flow_dir.join(node), path)
      }
      Self::External(path) => path.to_owned(),
    }
  }
}

/// Unlike [`Utf8Path::join`], joining an empty path doesn't add a trailing separator.
fn join(base: &Utf8Path, path: &Utf8Path) -> Utf8PathBuf {
  if path.as_str().is_empty() { base.to_owned() } else { base.join(path) }
}

#[derive(Debug, Clone, Error)]
#[error(
  "'{0}' isn't `<path>`, `./<path>`, `../<node>/<path>` or `/<path>`, with no `..` in `<path>`"
)]
pub struct InvalidPath(pub Utf8PathBuf);

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(path: &str) -> Result<NodePath<'_>, InvalidPath> {
    NodePath::parse(Utf8Path::new(path))
  }

  #[test]
  fn parses_kinds() {
    let p = Utf8Path::new;
    assert_eq!(parse("./top.f").unwrap(), NodePath::Local(p("top.f")));
    assert_eq!(parse("top.f").unwrap(), NodePath::Local(p("top.f")));
    assert_eq!(parse("obj_dir/Vtop").unwrap(), NodePath::Local(p("obj_dir/Vtop")));
    assert_eq!(parse("./obj_dir/Vtop").unwrap(), NodePath::Local(p("obj_dir/Vtop")));
    assert_eq!(parse(".").unwrap(), NodePath::Local(p("")));
    assert_eq!(
      parse("../flist/top.f").unwrap(),
      NodePath::OtherNode { node: "flist", path: p("top.f") }
    );
    assert_eq!(parse("../flist").unwrap(), NodePath::OtherNode { node: "flist", path: p("") });
    assert_eq!(parse("/src/a.sv").unwrap(), NodePath::External(p("/src/a.sv")));
  }

  #[test]
  fn ignores_what_components_normalize() {
    for path in ["a/./b", "a//b", "a/", "././a", "./"] {
      assert!(matches!(parse(path), Ok(NodePath::Local(_))), "{path} should be local");
    }
    for path in ["/a/./b", "//a", "/a/"] {
      assert!(matches!(parse(path), Ok(NodePath::External(_))), "{path} should be external");
    }
  }

  #[test]
  fn rejects_parent_dirs() {
    let invalid =
      ["", "a/../b", "..", "../", "../../x", "../a/../b", "../.", "./a/../b", "./../a", "/a/../b"];
    for path in invalid {
      assert!(parse(path).is_err(), "{path} should be invalid");
    }
  }

  #[test]
  fn resolves_against_node_dir() {
    let dir = Utf8Path::new("/flow/003/lint");
    let resolve = |path| parse(path).unwrap().resolve(dir);
    assert_eq!(resolve("./log"), "/flow/003/lint/log");
    assert_eq!(resolve("../flist/top.f"), "/flow/003/flist/top.f");
    assert_eq!(resolve("../flist"), "/flow/003/flist");
    assert_eq!(resolve("/src/a.sv"), "/src/a.sv");
  }
}
