use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::{FlowError, FlowResult, node::Node};
use crate::util::serialization;

/// The nodes of a flow.
#[derive(Default, Serialize, Deserialize)]
pub struct FlowDescription {
  pub nodes: BTreeMap<String, Node>,
}

#[derive(Serialize)]
struct NodeDeps {
  deps: BTreeSet<String>,
}

impl FlowDescription {
  pub async fn export(&self, path: &Utf8Path) -> FlowResult<()> {
    serialization::write(self, path).await.map_err(|err| FlowError::Serialization(err.into()))
  }

  pub async fn read(path: &Utf8Path) -> FlowResult<Self> {
    serialization::read(path).await.map_err(|err| FlowError::Serialization(err.into()))
  }

  /// The nodes each node depends on, which must be in the flow.
  pub fn deps(&self) -> FlowResult<BTreeMap<&str, BTreeSet<String>>> {
    let mut all_deps = BTreeMap::new();
    for (name, node) in &self.nodes {
      let deps = node.deps().map_err(|err| FlowError::NodeError(name.clone(), err.into()))?;
      if let Some(dep) = deps.iter().find(|dep| !self.nodes.contains_key(*dep)) {
        return Err(FlowError::NodeNotFound(dep.clone()));
      }
      all_deps.insert(name.as_str(), deps);
    }
    Ok(all_deps)
  }

  pub fn deps_toml(&self) -> FlowResult<String> {
    let deps: BTreeMap<_, _> =
      self.deps()?.into_iter().map(|(name, deps)| (name, NodeDeps { deps })).collect();
    Ok(toml::to_string(&deps).expect("The dependencies are expected to serialize."))
  }
}
