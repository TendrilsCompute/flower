#[cfg(not(unix))]
compile_error!("flower only supports Unix-like systems, such as Linux and macOS.");

pub mod flow;
pub mod util;
