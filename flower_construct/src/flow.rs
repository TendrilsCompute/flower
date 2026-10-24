use std::collections::btree_map::Entry;

use camino::{Utf8Path, Utf8PathBuf};
use flower::flow::{
  FlowDescription,
  node::{Node, NodeRunInfo},
};

use crate::{ConstructionError, ConstructionResult};

/// A node added to a [`FlowBuilder`], through which other nodes depend on it.
#[derive(Clone, Debug)]
pub struct NodeRef {
  dir: Utf8PathBuf,
}
impl NodeRef {
  /// A file the node writes in its directory.
  pub fn file(&self, path: impl AsRef<Utf8Path>) -> Utf8PathBuf {
    self.dir.join(path)
  }

  /// Requiring this orders after the node, rerunning whenever it does.
  pub fn done(&self) -> Utf8PathBuf {
    NodeRunInfo::manifest_path(&self.dir)
  }
}

/// Builds a [`FlowDescription`], handing out a [`NodeRef`] for each node added.
/// As nodes are referred to through these, they're added after their dependencies.
#[derive(Default)]
pub struct FlowBuilder {
  flow: FlowDescription,
}
impl FlowBuilder {
  pub fn add(&mut self, name: impl Into<String>, node: Node) -> ConstructionResult<NodeRef> {
    let name = name.into();
    // Other nodes are reached from a node's directory through its parent.
    let dir = Utf8Path::new("..").join(&name);
    match self.flow.nodes.entry(name) {
      Entry::Occupied(entry) => Err(ConstructionError::DuplicateNode(entry.key().clone())),
      Entry::Vacant(entry) => {
        entry.insert(node);
        Ok(NodeRef { dir })
      }
    }
  }

  /// Adds `node`, which runs only after each of `deps` has succeeded.
  pub fn add_after<'a>(
    &mut self,
    name: impl Into<String>,
    mut node: Node,
    deps: impl IntoIterator<Item = &'a NodeRef>,
  ) -> ConstructionResult<NodeRef> {
    node.required_paths.extend(deps.into_iter().map(NodeRef::done));
    self.add(name, node)
  }

  /// Adds a node without commands, which succeeds once all of `deps` have.
  pub fn collect<'a>(
    &mut self,
    name: impl Into<String>,
    deps: impl IntoIterator<Item = &'a NodeRef>,
  ) -> ConstructionResult<NodeRef> {
    self.add_after(name, Node { required_paths: vec![], commands: vec![] }, deps)
  }

  pub fn build(self) -> FlowDescription {
    self.flow
  }
}
