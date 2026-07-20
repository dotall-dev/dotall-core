use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use dotall_core::registry::FormatRegistry;
use dotall_core::{DotallStore, Engine, Result};

/// Runtime settings that affect behavior when an MCP session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushOnClose {
    enabled: bool,
}

impl FlushOnClose {
    pub fn resolve(no_flush_on_close: bool, environment_value: Option<&str>) -> Self {
        Self {
            enabled: !no_flush_on_close && environment_value != Some("0"),
        }
    }

    pub fn from_environment(no_flush_on_close: bool) -> Self {
        Self::resolve(
            no_flush_on_close,
            env::var("DOTALL_MCP_FLUSH_ON_CLOSE").ok().as_deref(),
        )
    }

    pub const fn enabled(self) -> bool {
        self.enabled
    }
}

impl Default for FlushOnClose {
    fn default() -> Self {
        Self::resolve(false, None)
    }
}

/// Command-line options shared by the MCP stdio entry point.
#[derive(Debug, Parser)]
#[command(name = "dotall-mcp")]
pub struct ServerOptions {
    /// Leave staged edits pending when the MCP session ends.
    #[arg(long)]
    pub no_flush_on_close: bool,
}

impl ServerOptions {
    pub fn flush_on_close(&self) -> FlushOnClose {
        FlushOnClose::from_environment(self.no_flush_on_close)
    }
}

/// Per-connection state shared by future MCP tool handlers.
#[derive(Clone)]
pub struct DotallServer {
    engine: Arc<Mutex<Engine>>,
    workspace_root: PathBuf,
    flush_on_close: FlushOnClose,
}

impl DotallServer {
    /// Opens an existing workspace or initializes project-local `.all/` storage.
    pub fn open_or_init(workspace: impl AsRef<Path>, flush_on_close: FlushOnClose) -> Result<Self> {
        let store = DotallStore::init(workspace)?;
        Ok(Self::from_store(store, flush_on_close))
    }

    pub fn from_store(store: DotallStore, flush_on_close: FlushOnClose) -> Self {
        let workspace_root = store.workspace().root().to_path_buf();
        Self {
            engine: Arc::new(Mutex::new(Engine::new(store, registry()))),
            workspace_root,
            flush_on_close,
        }
    }

    pub fn flush_on_close(&self) -> FlushOnClose {
        self.flush_on_close
    }

    pub fn engine(&self) -> Arc<Mutex<Engine>> {
        Arc::clone(&self.engine)
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

#[cfg(feature = "xlsx")]
pub fn registry() -> FormatRegistry {
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(dotall_xlsx::XlsxFormat));
    registry
}

#[cfg(not(feature = "xlsx"))]
pub fn registry() -> FormatRegistry {
    FormatRegistry::default()
}
