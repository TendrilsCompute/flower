pub mod command;
mod description;
pub mod node;
pub mod path;
mod run;

use camino::Utf8PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::io;

use crate::util::serialization::SerializationError;
pub use description::FlowDescription;
use node::NodeError;
pub use run::{FlowRunner, FlowTask, RunConfig};

fn io_err(path: impl Into<Utf8PathBuf>) -> impl FnOnce(io::Error) -> FlowError {
  let path = path.into();
  move |err| FlowError::Io(path, Arc::new(err))
}

type FlowResult<T> = Result<T, FlowError>;
#[derive(Clone, Debug, Error)]
pub enum FlowError {
  #[error("Node '{0}' could not be found.")]
  NodeNotFound(String),
  #[error("Circular dependency involving node '{0}'.")]
  CircularDependency(String),
  #[error("Node '{0}' failure: {1}")]
  NodeError(String, Arc<NodeError>),
  #[error("Io error involving {0}: {1}")]
  Io(Utf8PathBuf, Arc<io::Error>),
  #[error("{0} isn't a link to `../<run>/<node>`")]
  InvalidLink(Utf8PathBuf),
  #[error("Serialization: {0}")]
  Serialization(Arc<SerializationError>),
  #[error("Dependency '{0}' has no output at {1}.")]
  MissingDependency(String, Utf8PathBuf),
}
