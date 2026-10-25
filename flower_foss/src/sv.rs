use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
pub struct SystemVerilogLibrary {
  pub includes: Vec<Utf8PathBuf>,
  pub sources: Vec<Utf8PathBuf>,
}
impl SystemVerilogLibrary {
  pub fn paths_mut(&mut self) -> impl Iterator<Item = &mut Utf8PathBuf> {
    self.includes.iter_mut().chain(self.sources.iter_mut())
  }

  pub fn paths(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.includes.iter().chain(self.sources.iter())
  }
}

/// A top's parameter values, SystemVerilog expressions, by name.
pub type Params = BTreeMap<String, String>;

#[derive(Clone)]
pub struct SystemVerilogTop<'s> {
  pub top: &'s str,
  pub libs: Vec<&'s SystemVerilogLibrary>,
  pub params: Option<&'s Params>,
}
impl<'s> SystemVerilogTop<'s> {
  pub fn param_overrides(&self) -> impl Iterator<Item = String> {
    self.params.into_iter().flatten().map(|(name, value)| format!("{name}={value}"))
  }

  pub fn required_paths(&self) -> Vec<Utf8PathBuf> {
    self.libs.iter().flat_map(|l| l.paths()).cloned().collect()
  }

  pub fn include_paths(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.libs.iter().flat_map(|l| l.includes.iter())
  }

  pub fn source_paths(&self) -> impl Iterator<Item = &Utf8PathBuf> {
    self.libs.iter().flat_map(|l| l.sources.iter())
  }
}
