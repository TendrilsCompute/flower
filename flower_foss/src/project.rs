use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
  sv::SystemVerilogLibrary,
  tool::{cocotb::CocotbTests, yosys::FormalConfig},
};

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
  pub sv_lib: BTreeMap<String, SystemVerilogLibrary>,
  /// The cocotb tests of each top.
  #[serde(default)]
  pub cocotb: BTreeMap<String, CocotbTests>,
  /// The formal verification of each top.
  #[serde(default)]
  pub formal: BTreeMap<String, FormalConfig>,
}
impl Project {
  pub fn with_paths_canonicalized_on(mut self, project_dir: &Utf8Path) -> Self {
    let sv_paths = self.sv_lib.values_mut().flat_map(SystemVerilogLibrary::paths_mut);
    let cocotb_paths = self.cocotb.values_mut().flat_map(|tests| tests.test_modules.iter_mut());
    let formal_paths = self.formal.values_mut().flat_map(|formal| formal.props.iter_mut());
    for path in sv_paths.chain(cocotb_paths).chain(formal_paths) {
      *path = project_dir.join(&path);
    }
    self
  }
}
