use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::sv::SystemVerilogLibrary;

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
  pub sv_lib: BTreeMap<String, SystemVerilogLibrary>,
}
impl Project {
  pub fn with_paths_canonicalized_on(mut self, project_dir: &Utf8Path) -> Self {
    for path in self.sv_lib.values_mut().flat_map(SystemVerilogLibrary::paths_mut) {
      *path = project_dir.join(&path);
    }
    self
  }
}
