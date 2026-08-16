use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use dotall_core::registry::{
    Actor, ActorKind, Capability, FormatRegistry, ReadRequest, ReadSelector, SemanticOperation,
};
use dotall_core::{DotallError, DotallStore, EditRequest, Engine, Result as DotallResult};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde_json::json;
use uuid::Uuid;

use crate::params::{
    ApplyParams, CapabilitiesParams, DepsParams, DiffParams, DiscardParams, EditParams, FileParams,
    HistoryParams, InitParams, ReadParams, RevertParams, StagedParams, StatusParams,
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
    session: Arc<Mutex<SessionInner>>,
    flush_on_close: FlushOnClose,
}

struct SessionInner {
    engine: Engine,
    workspace_root: PathBuf,
}

impl SessionInner {
    fn from_store(store: DotallStore) -> Self {
        let workspace_root = store.workspace().root().to_path_buf();
        Self {
            engine: Engine::new(store, registry()),
            workspace_root,
        }
    }
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
        Self {
            session: Arc::new(Mutex::new(SessionInner::from_store(store))),
            flush_on_close,
        }
    }

    pub fn flush_on_close(&self) -> FlushOnClose {
        self.flush_on_close
    }

    /// Best-effort applies every pending transaction when the session closes.
    ///
    /// A failure for one file is reported to stderr and does not prevent other
    /// tracked files from being flushed.
    pub fn flush_staged(&self) {
        if !self.flush_on_close.enabled() {
            return;
        }

        let session = self.session();
        let Ok(mut session) = session.lock() else {
            eprintln!("dotall-mcp: unable to flush staged edits: session lock is poisoned");
            return;
        };

        let tracked = match session.engine.status() {
            Ok(tracked) => tracked,
            Err(error) => {
                eprintln!("dotall-mcp: unable to enumerate staged edits: {error}");
                return;
            }
        };

        for object in tracked {
            let staged = match session.engine.staged(&object.path) {
                Ok(staged) => staged,
                Err(error) => {
                    eprintln!(
                        "dotall-mcp: unable to inspect staged edits for {}: {error}",
                        object.path
                    );
                    continue;
                }
            };
            if staged.is_empty() {
                continue;
            }

            if let Err(error) = session.engine.apply_all(&object.path) {
                eprintln!(
                    "dotall-mcp: unable to flush staged edits for {}: {error}",
                    object.path
                );
            }
        }
    }

    fn session(&self) -> Arc<Mutex<SessionInner>> {
        Arc::clone(&self.session)
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
        let server = self.clone();
        Json(
            blocking(move || {
                let workspace = canonical_workspace(&params.workspace)?;
                let initialized = !workspace.join(".all/manifest.json").is_file();
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                if session.workspace_root != workspace {
                    *session = SessionInner::from_store(DotallStore::init(&workspace)?);
                }
                json_result(json!({
                    "workspace": session.workspace_root,
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
        let server = self.clone();
        Json(
            blocking(move || {
                let workspace = canonical_workspace(&params.workspace)?;
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                if session.workspace_root != workspace {
                    *session = SessionInner::from_store(DotallStore::open(&workspace)?);
                }
                let objects = session.engine.status()?;
                json_result(json!({
                    "workspace": session.workspace_root,
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
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                json_result(
                    serde_json::to_value(session.engine.inspect(&relative)?)
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
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                let inspection = session.engine.inspect(&relative)?;
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
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                json_result(
                    serde_json::to_value(session.engine.read(&relative, &request)?)
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
        Json(
            blocking(move || {
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                json_result(
                    session
                        .engine
                        .deps(&relative, &params.cell, params.dependents)?,
                )
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_edit",
        description = "Stage semantic edit operations only; this never applies source changes. Call capabilities or inspect first, use its operation names and payload examples, supply its required source hash, and reuse transaction_id for retries. Call staged then apply, or use the exact revert route returned after apply."
    )]
    pub async fn dotall_edit(
        &self,
        Parameters(params): Parameters<EditParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                let request = EditRequest {
                    transaction_id: optional_transaction_id(params.transaction_id)?,
                    expected_source_hash: params.expected_source_hash,
                    actor: Actor {
                        kind: ActorKind::Mcp,
                        id: Some(params.actor_id),
                    },
                    operations: params
                        .operations
                        .into_iter()
                        .map(|operation| SemanticOperation {
                            kind: operation.kind,
                            payload: operation.payload,
                        })
                        .collect(),
                };
                json_result(
                    serde_json::to_value(session.engine.edit(&relative, &request)?)
                        .map_err(serialization_error)?,
                )
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_staged",
        description = "List staged transactions for a file. Staged edits have not changed the source; apply one, apply all, discard, or leave them for flush-on-close."
    )]
    pub async fn dotall_staged(
        &self,
        Parameters(params): Parameters<StagedParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                json_result(json!({ "edits": session.engine.staged(&relative)? }))
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_apply",
        description = "Apply exactly one staged transaction or all staged transactions for a file, creating immutable versions. Set exactly one of transaction_id or all: true."
    )]
    pub async fn dotall_apply(
        &self,
        Parameters(params): Parameters<ApplyParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                match (params.transaction_id, params.all) {
                    (Some(transaction_id), false) => {
                        let transaction_id = parse_transaction_id(&transaction_id, "<apply>")?;
                        json_result(
                            serde_json::to_value(session.engine.apply(&relative, transaction_id)?)
                                .map_err(serialization_error)?,
                        )
                    }
                    (None, true) => json_result(json!({
                        "applied": session.engine.apply_all(&relative)?,
                    })),
                    _ => Err(invalid_request(
                        "<apply>",
                        "apply requires exactly one of transaction_id or all: true",
                    )),
                }
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_discard",
        description = "Discard one staged transaction without changing source bytes or committed history."
    )]
    pub async fn dotall_discard(
        &self,
        Parameters(params): Parameters<DiscardParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                let transaction_id = parse_transaction_id(&params.transaction_id, "<discard>")?;
                session.engine.discard(&relative, transaction_id)?;
                json_result(json!({ "discarded": transaction_id }))
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_history",
        description = "List immutable committed versions for a file, including actor, hashes, and semantic-change summaries."
    )]
    pub async fn dotall_history(
        &self,
        Parameters(params): Parameters<HistoryParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                json_result(json!({ "entries": session.engine.history(&relative)? }))
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_diff",
        description = "Return the ordered semantic changes recorded for one committed version."
    )]
    pub async fn dotall_diff(
        &self,
        Parameters(params): Parameters<DiffParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                json_result(
                    serde_json::to_value(session.engine.diff(&relative, params.version)?)
                        .map_err(serialization_error)?,
                )
            })
            .await,
        )
    }

    #[tool(
        name = "dotall_revert",
        description = "Stage restoration of a committed version's pre-edit snapshot. This does not apply source changes: call apply with the returned transaction_id, or let flush-on-close apply it."
    )]
    pub async fn dotall_revert(
        &self,
        Parameters(params): Parameters<RevertParams>,
    ) -> Json<ToolResponse<JsonResult>> {
        let server = self.clone();
        Json(
            blocking(move || {
                let session = server.session();
                let mut session = session.lock().map_err(lock_error)?;
                let relative = relative_path_for_file(&session.workspace_root, params.file)?;
                let current = session.engine.inspect(&relative)?.source_hash;
                if current != params.expected_source_hash {
                    return Err(DotallError::SourceHashMismatch {
                        path: PathBuf::from(&relative),
                        expected: params.expected_source_hash,
                        actual: current,
                    });
                }
                let transaction_id = optional_transaction_id(params.transaction_id)?;
                json_result(
                    serde_json::to_value(session.engine.revert(
                        &relative,
                        params.version,
                        transaction_id,
                    )?)
                    .map_err(serialization_error)?,
                )
            })
            .await,
        )
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

fn optional_transaction_id(transaction_id: Option<String>) -> DotallResult<Uuid> {
    match transaction_id {
        Some(transaction_id) => parse_transaction_id(&transaction_id, "<transaction_id>"),
        None => Ok(Uuid::new_v4()),
    }
}

fn parse_transaction_id(transaction_id: &str, path: &str) -> DotallResult<Uuid> {
    Uuid::parse_str(transaction_id)
        .map_err(|_| invalid_request(path, "transaction_id must be a UUID"))
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> DotallError {
    invalid_request("<session>", "MCP engine session lock is poisoned")
}

fn canonical_workspace(workspace: &str) -> DotallResult<PathBuf> {
    let path = Path::new(workspace);
    path.canonicalize().map_err(|source| DotallError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub fn registry() -> FormatRegistry {
    let mut registry = FormatRegistry::default();
    #[cfg(feature = "xlsx")]
    registry.register(Arc::new(dotall_xlsx::XlsxFormat));
    #[cfg(feature = "pptx")]
    registry.register(Arc::new(dotall_pptx::PptxFormat));
    #[cfg(feature = "docx")]
    registry.register(Arc::new(dotall_docx::DocxFormat));
    #[cfg(feature = "pdf")]
    registry.register(Arc::new(dotall_pdf::PdfFormat));
    registry
}
