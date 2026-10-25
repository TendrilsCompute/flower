use camino::Utf8Path;
use flower_construct::{ConstructionResult, path::canonicalize_on};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
  sv::{Params, SystemVerilogLibrary},
  tool::{cocotb::CocotbTests, yosys::FormalConfig},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
  pub sv_lib: BTreeMap<String, SystemVerilogLibrary>,
  /// The configuration of each top. A top without one is only linted and built.
  #[serde(default)]
  pub top: BTreeMap<String, Top>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Top {
  /// Parameters, overriding their defaults.
  #[serde(default)]
  pub params: Params,
  pub cocotb: Option<CocotbTests>,
  pub formal: Option<FormalConfig>,
  #[serde(default)]
  pub verilator_testbench: bool,
}
impl Project {
  pub async fn with_paths_canonicalized_on(
    mut self,
    project_dir: &Utf8Path,
  ) -> ConstructionResult<Self> {
    let sv_paths = self.sv_lib.values_mut().flat_map(SystemVerilogLibrary::paths_mut);
    let top_paths = self.top.values_mut().flat_map(|top| {
      let cocotb_paths = top.cocotb.iter_mut().flat_map(|tests| tests.test_modules.iter_mut());
      let formal_paths = top.formal.iter_mut().flat_map(|formal| formal.props.iter_mut());
      cocotb_paths.chain(formal_paths)
    });
    for path in sv_paths.chain(top_paths) {
      *path = canonicalize_on(path, project_dir).await?;
    }
    Ok(self)
  }
}
