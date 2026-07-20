use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use dotall_core::registry::{Capability, FormatRegistry, ReadRequest, ReadSelector};
use dotall_core::{DotallError, DotallStore, Engine, Result as DotallResult};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde_json::json;

use crate::params::{
    CapabilitiesParams, DepsParams, FileParams, InitParams, ReadParams, StatusParams,
};
use crate::response::{JsonResult, ToolResponse};
use crate::tools::{blocking, relative_path_for_file};

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
    pub fn open_or_init(
        workspace: impl AsRef<Path>,
        flush_on_close: FlushOnClose,
    ) -> DotallResult<Self> {
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

#[tool_router]
impl DotallServer {
    #[tool(
        name = "dotall_init",
        description = "Initialize project-local .all/ storage. Safe and idempotent; call capabilities or inspect before edit."
    )]
    pub async fn dotall_init(
        &self,
        Parameters(params): Parameters<InitParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        Json(
            blocking(move || {
                let path = PathBuf::from(params.workspace);
                let initialized = !path.join(".all/manifest.json").is_file();
                let store = DotallStore::init(&path)?;
                json_result(json!({
                    "workspace": store.workspace().root(),
                    "initialized": initialized,
                }))
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_status",
        description = "Report tracked files, freshness, source hashes, and versions. Call capabilities or inspect before edit."
    )]
    pub async fn dotall_status(
        &self,
        Parameters(params): Parameters<StatusParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        Json(
            blocking(move || {
                let store = DotallStore::open(params.workspace)?;
                let objects = store.status()?;
                let objects = objects
                    .into_iter()
                    .map(|object| {
                        let tracked = &store.manifest().objects[&object.path];
                        json!({
                            "path": object.path,
                            "format_id": object.format_id,
                            "state": object.state,
                            "source_hash": tracked.fingerprint.blake3,
                            "version_count": tracked.version_count,
                        })
                    })
                    .collect::<Vec<_>>();
                json_result(json!({
                    "workspace": store.workspace().root(),
                    "tracked_count": objects.len(),
                    "objects": objects,
                }))
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_inspect",
        description = "Inspect a file before reading or editing. Returns summary, current source hash, supported selectors and semantic edit operations, plus suggested next reads."
    )]
    pub async fn dotall_inspect(
        &self,
        Parameters(params): Parameters<FileParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let relative = relative_path_for_file(&server, params.file)?;
                let engine = server.engine();
                let mut engine = engine.lock().map_err(lock_error)?;
                json_result(
                    serde_json::to_value(engine.inspect(&relative)?)
                        .map_err(serialization_error)?,
                )
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_capabilities",
        description = "Discover a file's format, read selectors, and supported edit operations before edit. Call this or inspect before every edit."
    )]
    pub async fn dotall_capabilities(
        &self,
        Parameters(params): Parameters<CapabilitiesParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let relative = relative_path_for_file(&server, params.file)?;
                let engine = server.engine();
                let mut engine = engine.lock().map_err(lock_error)?;
                let inspection = engine.inspect(&relative)?;
                let selectors = inspection
                    .inspection
                    .capabilities
                    .iter()
                    .filter_map(|capability| match capability {
                        Capability::ReadFull => Some("full".to_owned()),
                        Capability::ReadSelector { kind } => Some(kind.clone()),
                        Capability::Inspect => None,
                    })
                    .collect::<Vec<_>>();
                json_result(json!({
                    "format_id": inspection.inspection.format_id,
                    "selectors": selectors,
                    "edit_capabilities": inspection.inspection.edit_capabilities,
                    "suggested_reads": inspection.inspection.suggested_reads,
                }))
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_read",
        description = "Read a token-budgeted file projection. Use inspect or capabilities before edit, and reuse returned continuation cursors."
    )]
    pub async fn dotall_read(
        &self,
        Parameters(params): Parameters<ReadParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let relative = relative_path_for_file(&server, params.file)?;
                let selector = match (params.selector_kind, params.selector) {
                    (Some(kind), Some(value)) => Some(ReadSelector { kind, value }),
                    (None, None) => None,
                    (Some(_), None) => {
                        return Err(invalid_request(
                            "<read>",
                            "selector_kind requires a selector value",
                        ));
                    }
                    (None, Some(_)) => {
                        return Err(invalid_request("<read>", "selector requires selector_kind"));
                    }
                };
                let request = ReadRequest {
                    selector,
                    max_tokens: params.max_tokens.unwrap_or(1_500),
                    continuation: params.continuation,
                };
                let engine = server.engine();
                let mut engine = engine.lock().map_err(lock_error)?;
                json_result(
                    serde_json::to_value(engine.read(&relative, &request)?)
                        .map_err(serialization_error)?,
                )
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_deps",
        description = "Query formula precedents or dependents for an XLSX cell. Inspect or capabilities before edit."
    )]
    pub async fn dotall_deps(
        &self,
        Parameters(params): Parameters<DepsParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(blocking(move || deps(&server, params)).await)
    }
}

#[tool_handler(
    name = "dotall",
    version = "0.1.0",
    instructions = "Call capabilities or inspect before edit. Reuse continuation cursors and transaction IDs. Never bypass an unsupported capability with ad hoc file mutation."
)]
impl ServerHandler for DotallServer {}

fn json_result(data: serde_json::Value) -> DotallResult<JsonResult> {
    Ok(JsonResult { data })
}

fn serialization_error(source: serde_json::Error) -> DotallError {
    DotallError::Serialization {
        context: "MCP tool response".into(),
        source,
    }
}

fn invalid_request(path: impl Into<PathBuf>, reason: impl Into<String>) -> DotallError {
    DotallError::InvalidSourcePath {
        path: path.into(),
        reason: reason.into(),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> DotallError {
    invalid_request("<session>", "MCP engine session lock is poisoned")
}

#[cfg(feature = "xlsx")]
fn deps(server: &DotallServer, params: DepsParams) -> DotallResult<JsonResult> {
    use dotall_xlsx::WorkbookModel;
    use dotall_xlsx::dependencies::{DependencyDirection, ensure_and_query};

    let relative = relative_path_for_file(server, &params.file)?;
    let engine = server.engine();
    let mut engine = engine.lock().map_err(lock_error)?;
    let model = engine.load_model(&relative)?;
    drop(engine);

    let workbook: WorkbookModel =
        serde_json::from_value(model.envelope.payload.clone()).map_err(|source| {
            DotallError::Serialization {
                context: "XLSX workbook artifact payload".into(),
                source,
            }
        })?;
    let element_id = cell_element_id(&workbook, &params.cell)?;
    let direction = if params.dependents {
        DependencyDirection::Reverse
    } else {
        DependencyDirection::Forward
    };
    let store = DotallStore::open(server.workspace_root())?;
    let result = ensure_and_query(
        &store,
        &relative,
        &model.envelope,
        &model.source_hash,
        &element_id,
        direction,
    )?;
    json_result(serde_json::to_value(result).map_err(serialization_error)?)
}

#[cfg(not(feature = "xlsx"))]
fn deps(_server: &DotallServer, _params: DepsParams) -> DotallResult<JsonResult> {
    Err(DotallError::UnsupportedCapability {
        format_id: "dotall-mcp".into(),
        capability: "deps".into(),
        available: vec!["Rebuild dotall-mcp with the xlsx feature.".into()],
    })
}

#[cfg(feature = "xlsx")]
fn cell_element_id(workbook: &dotall_xlsx::WorkbookModel, selector: &str) -> DotallResult<String> {
    let (sheet_name, address) =
        selector
            .rsplit_once('!')
            .ok_or_else(|| DotallError::UnsupportedCapability {
                format_id: "xlsx".into(),
                capability: "invalid cell selector".into(),
                available: vec!["use Sheet!A1".into()],
            })?;
    let address = address.to_ascii_uppercase();
    workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(sheet_name))
        .and_then(|sheet| {
            sheet
                .cells
                .iter()
                .find(|cell| cell.address.eq_ignore_ascii_case(&address))
        })
        .map(|cell| cell.element_id.clone())
        .ok_or_else(|| DotallError::UnsupportedCapability {
            format_id: "xlsx".into(),
            capability: format!("unknown cell selector {selector}"),
            available: vec!["use an existing Sheet!A1 cell address".into()],
        })
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
