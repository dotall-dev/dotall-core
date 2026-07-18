mod error;
mod workspace;

pub use error::{DotallError, Result};
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
