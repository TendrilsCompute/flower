use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{sv::SystemVerilogLibrary, tool::cocotb::CocotbTests};

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
  pub sv_lib: BTreeMap<String, SystemVerilogLibrary>,
  /// The cocotb tests of each top.
  #[serde(default)]
  pub cocotb: BTreeMap<String, CocotbTests>,
}
impl Project {
  pub fn with_paths_canonicalized_on(mut self, project_dir: &Utf8Path) -> Self {
    let sv_paths = self.sv_lib.values_mut().flat_map(SystemVerilogLibrary::paths_mut);
    let cocotb_paths = self.cocotb.values_mut().flat_map(|tests| tests.test_modules.iter_mut());
    for path in sv_paths.chain(cocotb_paths) {
      *path = project_dir.join(&path);
    }
    self
  }
}
