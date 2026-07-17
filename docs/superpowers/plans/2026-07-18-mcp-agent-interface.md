# MCP Agent Interface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the completed Dotall core as a local stdio MCP server whose discoverable tools let agents inspect, read, edit, version, diff, and revert files without format-specific code.

**Architecture:** Add a thin feature-gated `dotall-mcp` binary using the official `rmcp` Rust SDK. MCP parameter types translate directly into core request types; handlers execute blocking file operations through `spawn_blocking` and return structured, agent-recoverable results. The MCP crate contains no parser, cache, history, or write logic.

**Tech Stack:** Stable Rust, `rmcp` official SDK (current verified release on 2026-07-18: 2.2.0), Tokio, `schemars`, `serde`, `serde_json`, existing Dotall core/XLSX crates, and official rmcp client transport for integration tests.

---

## Preconditions and protocol rule

Execute the core foundation, XLSX read, dependency, and transactional cell-edit
plans first. Broader XLSX operations can be added before or after MCP because
capabilities are discovered dynamically.

At implementation time, run `cargo add` to select the latest compatible official
SDK rather than hand-writing a version. If its API differs from the verified 2.2.0
API used below, update only the transport/macro adapter and preserve tool schemas
and behavior.

Stdout is exclusively MCP protocol traffic. Diagnostics and tracing go to stderr.
Commit steps require explicit user authorization.

## Target files

```text
crates/dotall-mcp/
├── Cargo.toml
├── src/
│   ├── error.rs
│   ├── lib.rs
│   ├── main.rs
│   ├── params.rs
│   ├── response.rs
│   ├── server.rs
│   └── tools/
│       ├── edit.rs
│       ├── inspect.rs
│       ├── mod.rs
│       ├── read.rs
│       └── versions.rs
└── tests/
    ├── schemas.rs
    └── stdio.rs
```

### Task 1: Bootstrap a feature-gated stdio MCP binary

**Files:**

- Create: `crates/dotall-mcp/Cargo.toml`
- Create: `crates/dotall-mcp/src/lib.rs`
- Create: `crates/dotall-mcp/src/main.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Create the crate and add current official dependencies**

Run:

```bash
cargo new --bin crates/dotall-mcp --vcs none
cargo add --package dotall-mcp dotall-core --path crates/dotall-core
cargo add --package dotall-mcp dotall-xlsx --path crates/dotall-xlsx --optional
cargo add --package dotall-mcp rmcp --features server,macros,transport-io
cargo add --package dotall-mcp tokio --features macros,rt-multi-thread
cargo add --package dotall-mcp serde --features derive
cargo add --package dotall-mcp serde_json
cargo add --package dotall-mcp schemars
```

Add `crates/dotall-mcp` to workspace members and add:

```toml
[features]
default = ["xlsx"]
xlsx = ["dep:dotall-xlsx"]
```

- [ ] **Step 2: Add a minimal server smoke test**

Create `src/lib.rs`:

```rust
pub const SERVER_NAME: &str = "dotall";

#[cfg(test)]
mod tests {
    use super::SERVER_NAME;

    #[test]
    fn server_name_is_stable() {
        assert_eq!(SERVER_NAME, "dotall");
    }
}
```

Keep `main.rs` temporarily:

```rust
fn main() {
    eprintln!("dotall-mcp is not wired yet");
}
```

- [ ] **Step 3: Verify slim and default builds**

```bash
cargo check -p dotall-mcp
cargo check -p dotall-mcp --no-default-features
cargo test -p dotall-mcp
```

Expected: all commands succeed.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add Cargo.toml Cargo.lock crates/dotall-mcp
git commit -m "build: add feature-gated Dotall MCP crate"
```

### Task 2: Define stable MCP parameter and response schemas

**Files:**

- Create: `crates/dotall-mcp/src/params.rs`
- Create: `crates/dotall-mcp/src/response.rs`
- Create: `crates/dotall-mcp/src/error.rs`
- Modify: `crates/dotall-mcp/src/lib.rs`
- Create: `crates/dotall-mcp/tests/schemas.rs`

- [ ] **Step 1: Define common request parameters**

Create `params.rs`:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct InitParams {
    #[schemars(description = "Absolute or current-working-directory-relative workspace path")]
    pub workspace: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FileParams {
    #[schemars(description = "Path to a file inside an initialized Dotall workspace")]
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadParams {
    pub file: String,
    #[schemars(description = "Selector kind advertised by inspect, such as range, sheet, full, or dependencies")]
    pub selector_kind: Option<String>,
    #[schemars(description = "Format-natural selector, such as Revenue!A1:D20")]
    pub selector: Option<String>,
    #[schemars(range(min = 1, max = 100000))]
    pub max_tokens: Option<usize>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EditParams {
    pub file: String,
    #[schemars(description = "Source hash returned by inspect/status; prevents stale writes")]
    pub expected_source_hash: String,
    #[schemars(description = "Stable UUID reused when retrying the same edit")]
    pub transaction_id: Option<String>,
    pub actor_id: String,
    pub operations: Vec<OperationParam>,
    #[serde(default)]
    pub stage_only: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct OperationParam {
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HistoryParams {
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DiffParams {
    pub file: String,
    pub from_version: u64,
    pub to_version: u64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RevertParams {
    pub file: String,
    pub version: u64,
    pub expected_source_hash: String,
    pub transaction_id: Option<String>,
    pub actor_id: String,
}
```

- [ ] **Step 2: Define recoverable response envelopes**

Create `response.rs`:

```rust
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolResponse<T> {
    Success {
        result: T,
        next_actions: Vec<String>,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
        next_actions: Vec<String>,
        details: serde_json::Value,
    },
}

impl<T> ToolResponse<T> {
    pub fn success(result: T, next_actions: Vec<String>) -> Self {
        Self::Success {
            result,
            next_actions,
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct JsonResult {
    pub data: serde_json::Value,
}
```

- [ ] **Step 3: Map core errors without losing recovery data**

Create `error.rs`:

```rust
use dotall_core::DotallError;

use crate::response::{JsonResult, ToolResponse};

pub fn tool_error(error: DotallError) -> ToolResponse<JsonResult> {
    match error {
        DotallError::StaleSource {
            path,
            expected,
            actual,
        } => ToolResponse::Error {
            code: "stale_source".into(),
            message: format!(
                "{} changed since the agent inspected it",
                path.display()
            ),
            retryable: true,
            next_actions: vec![
                "Inspect the file again.".into(),
                "Rebuild the edit with the new source hash.".into(),
            ],
            details: serde_json::json!({
                "expected_hash": expected,
                "actual_hash": actual,
            }),
        },
        DotallError::UnsupportedCapability {
            format_id,
            capability,
            available,
        } => ToolResponse::Error {
            code: "unsupported_capability".into(),
            message: format!("{format_id} does not support {capability}"),
            retryable: false,
            next_actions: available,
            details: serde_json::json!({"format_id": format_id}),
        },
        other => ToolResponse::Error {
            code: "dotall_error".into(),
            message: other.to_string(),
            retryable: false,
            next_actions: vec!["Inspect the file or workspace status.".into()],
            details: serde_json::json!({}),
        },
    }
}
```

Extend this mapping exhaustively for transaction conflict, write lock, recovery
required, unsupported format, invalid selector, corrupt source, and output
validation. Every variant gets a stable snake-case code and useful next action.

- [ ] **Step 4: Assert generated schema requirements**

Create `tests/schemas.rs`:

```rust
use dotall_mcp::params::{EditParams, ReadParams};

#[test]
fn edit_schema_requires_stale_write_guard_and_operations() {
    let schema = schemars::schema_for!(EditParams);
    let json = serde_json::to_value(schema).expect("schema JSON");
    let required = json["required"].as_array().expect("required");

    assert!(required.iter().any(|value| value == "expected_source_hash"));
    assert!(required.iter().any(|value| value == "operations"));
}

#[test]
fn read_schema_exposes_budget_and_continuation() {
    let json = serde_json::to_value(schemars::schema_for!(ReadParams))
        .expect("schema JSON");
    let properties = json["properties"].as_object().expect("properties");

    assert!(properties.contains_key("max_tokens"));
    assert!(properties.contains_key("continuation"));
}
```

Export `params` and `response` from `lib.rs`.

- [ ] **Step 5: Run schema tests**

```bash
cargo test -p dotall-mcp --test schemas
```

Expected: required edit guards and progressive-read fields appear in JSON Schema.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add crates/dotall-mcp
git commit -m "feat(mcp): define agent-facing tool schemas"
```

### Task 3: Add a reusable core adapter

**Files:**

- Create: `crates/dotall-mcp/src/tools/mod.rs`
- Modify: `crates/dotall-mcp/src/lib.rs`

- [ ] **Step 1: Compose the registry outside core**

Create:

```rust
use std::path::{Path, PathBuf};

use dotall_core::{DotallStore, Engine, FormatRegistry, Result};

pub fn engine_for_file(file: &str) -> Result<(Engine, String)> {
    let file = PathBuf::from(file);
    let canonical = file
        .canonicalize()
        .map_err(|source| dotall_core::DotallError::io(&file, source))?;
    let parent = canonical.parent().ok_or_else(|| {
        dotall_core::DotallError::InvalidSourcePath {
            path: canonical.clone(),
            reason: "file has no parent directory".into(),
        }
    })?;
    let store = DotallStore::open(parent)?;
    let relative = canonical
        .strip_prefix(store.workspace().root())
        .map_err(|_| dotall_core::DotallError::InvalidSourcePath {
            path: canonical.clone(),
            reason: "file is outside the Dotall workspace".into(),
        })?
        .to_str()
        .ok_or_else(|| dotall_core::DotallError::InvalidSourcePath {
            path: canonical.clone(),
            reason: "non-UTF-8 paths are not supported".into(),
        })?
        .replace(std::path::MAIN_SEPARATOR, "/");

    Ok((Engine::new(store, registry()), relative))
}

pub fn registry() -> FormatRegistry {
    let mut registry = FormatRegistry::default();
    #[cfg(feature = "xlsx")]
    registry.register(std::sync::Arc::new(dotall_xlsx::XlsxFormat));
    registry
}
```

If `DotallError::io` remains crate-private after foundation execution, add a public
`DotallError::from_io(path, source)` constructor in core rather than duplicating an
I/O error type in MCP.

- [ ] **Step 2: Add blocking-operation helper**

```rust
pub async fn blocking<T, F>(operation: F) -> crate::response::ToolResponse<T>
where
    T: Send + 'static,
    F: FnOnce() -> dotall_core::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(Ok(value)) => crate::response::ToolResponse::success(value, Vec::new()),
        Ok(Err(error)) => crate::error::tool_error_typed(error),
        Err(error) => crate::response::ToolResponse::Error {
            code: "worker_failed".into(),
            message: error.to_string(),
            retryable: true,
            next_actions: vec!["Retry the same idempotent request.".into()],
            details: serde_json::json!({}),
        },
    }
}
```

Make `tool_error_typed<T>` generic so every tool shares one mapping.

- [ ] **Step 3: Verify no MCP logic entered core**

Run:

```bash
rg "rmcp|Model Context Protocol" crates/dotall-core
```

Expected: no matches.

### Task 4: Implement inspect, read, and status tools

**Files:**

- Create: `crates/dotall-mcp/src/tools/inspect.rs`
- Create: `crates/dotall-mcp/src/tools/read.rs`
- Create: `crates/dotall-mcp/src/server.rs`
- Modify: `crates/dotall-mcp/src/tools/mod.rs`
- Modify: `crates/dotall-mcp/src/lib.rs`

- [ ] **Step 1: Define server and tool router**

Create `server.rs`:

```rust
use rmcp::{tool_router, ServerHandler};

#[derive(Clone, Default)]
pub struct DotallServer;

#[tool_router]
impl DotallServer {}

#[rmcp::tool_handler(
    name = "dotall",
    version = env!("CARGO_PKG_VERSION"),
    instructions = "Inspect before reading or editing. Reuse continuation cursors and transaction IDs. Never bypass an unsupported capability with ad hoc file mutation."
)]
impl ServerHandler for DotallServer {}
```

As each tool is added, place its `#[tool]` method inside the routed impl.

- [ ] **Step 2: Implement `dotall_init` and `dotall_status`**

Tool descriptions:

```text
dotall_init: Initialize project-local .all/ storage. Safe and idempotent.
dotall_status: Report tracked files, freshness, source hashes, and versions.
```

`dotall_status` returns JSON data directly from core and suggests `dotall_inspect`
for stale or unknown files.

- [ ] **Step 3: Implement `dotall_inspect`**

Use:

```rust
#[rmcp::tool(
    description = "Inspect a file before reading or editing. Returns summary, current source hash, supported selectors and semantic edit operations, plus suggested next reads."
)]
pub async fn dotall_inspect(
    &self,
    rmcp::handler::server::wrapper::Parameters(params):
        rmcp::handler::server::wrapper::Parameters<FileParams>,
) -> rmcp::handler::server::wrapper::Json<ToolResponse<JsonResult>> {
    let response = blocking(move || {
        let (mut engine, relative) = engine_for_file(&params.file)?;
        let inspection = engine.inspect(&relative)?;
        Ok(JsonResult {
            data: serde_json::to_value(inspection).map_err(|source| {
                dotall_core::DotallError::Serialization {
                    context: "MCP inspection".into(),
                    source,
                }
            })?,
        })
    }).await;
    rmcp::handler::server::wrapper::Json(response)
}
```

- [ ] **Step 4: Implement `dotall_read`**

Translate optional selector fields into `ReadRequest`. Default `max_tokens` to
1,500. Reject a selector value without a selector kind. Return content,
estimated tokens, truncation, continuation, freshness/provenance, and suggested
next actions.

- [ ] **Step 5: Add direct handler tests**

Call each async method without stdio using a generated workbook. Assert:

- inspect advertises `range` and semantic edit operations;
- read defaults to useful summary/preview behavior;
- a small budget returns a continuation;
- status includes source hash and version count;
- unsupported selector returns `status: error` with available selectors.

Run:

```bash
cargo test -p dotall-mcp inspect
cargo test -p dotall-mcp read
```

Expected: direct tool tests pass.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add crates/dotall-mcp
git commit -m "feat(mcp): expose inspect read and status tools"
```

### Task 5: Implement edit, history, diff, and revert tools

**Files:**

- Create: `crates/dotall-mcp/src/tools/edit.rs`
- Create: `crates/dotall-mcp/src/tools/versions.rs`
- Modify: `crates/dotall-mcp/src/server.rs`

- [ ] **Step 1: Implement `dotall_edit`**

Translate operation parameters to core `SemanticOperation`, parse or generate the
transaction UUID, set actor kind `agent`, and call `Engine::edit`.

Tool description must state:

- inspect first;
- expected hash is mandatory;
- transaction ID must be reused on retry;
- operation names/payloads come from inspect;
- success returns an exact revert route.

- [ ] **Step 2: Implement version tools**

```text
dotall_history: immutable committed versions with actor, hashes, and semantic diffs
dotall_diff: ordered semantic changes between two versions
dotall_revert: restore a selected pre-edit state as a new version
```

Revert requires current expected hash, actor ID, and optional retry transaction ID.

- [ ] **Step 3: Test autonomous recovery behavior**

Direct handler tests must cover:

- successful edit result includes version/hash/revert action;
- identical retry returns idempotent replay;
- stale hash returns actual hash and inspect/retry actions;
- unsupported operation returns advertised alternatives;
- history is ordered;
- revert creates a new version.

- [ ] **Step 4: Verify**

```bash
cargo test -p dotall-mcp edit
cargo test -p dotall-mcp versions
```

Expected: all agent-recovery and version tests pass.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-mcp
git commit -m "feat(mcp): expose transactional edit and version tools"
```

### Task 6: Wire stdio without protocol pollution

**Files:**

- Replace: `crates/dotall-mcp/src/main.rs`
- Create: `crates/dotall-mcp/tests/stdio.rs`
- Modify: `crates/dotall-mcp/Cargo.toml`

- [ ] **Step 1: Implement official stdio transport**

Replace `main.rs`:

```rust
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = dotall_mcp::server::DotallServer
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
```

No `println!` or stdout logger is permitted anywhere in `dotall-mcp`.

- [ ] **Step 2: Add official client test dependencies**

Enable rmcp client and child-process transport for dev tests using `cargo add
--dev`, plus Tokio process features required by the current SDK.

- [ ] **Step 3: Test protocol initialization and discovery**

Spawn the built `dotall-mcp` binary with the official rmcp child-process client.
Initialize, call `list_tools`, and assert exact tool names:

```text
dotall_init
dotall_status
dotall_inspect
dotall_read
dotall_edit
dotall_history
dotall_diff
dotall_revert
```

Assert every tool has a non-empty description and object JSON schema.

- [ ] **Step 4: Test a full stdio agent session**

Through the client:

1. initialize a temporary workspace;
2. inspect a generated workbook;
3. read a range;
4. edit a formula using the returned source hash;
5. list history;
6. revert using the new hash;
7. verify the original formula.

Capture server stdout only through the protocol parser. Any non-protocol line fails
the test.

- [ ] **Step 5: Verify**

```bash
cargo test -p dotall-mcp --test stdio
```

Expected: list-tools and full-session tests pass without protocol pollution.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add crates/dotall-mcp
git commit -m "feat(mcp): serve Dotall tools over stdio"
```

### Task 7: Document installation and run the release gate

**Files:**

- Modify: `docs/specs/xlsx-engine-v0.md`
- Create: `docs/guides/mcp.md`
- Modify only code required to correct verification failures.

- [ ] **Step 1: Write configuration guide**

Document:

```json
{
  "mcpServers": {
    "dotall": {
      "command": "/absolute/path/to/dotall-mcp"
    }
  }
}
```

Include build/install commands, stdio-only behavior, initialization, inspect-first
workflow, expected-hash edits, idempotent retries, logs on stderr, and
troubleshooting for unsupported format or uninitialized workspace.

- [ ] **Step 2: Run all checks**

```bash
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p dotall-mcp --no-default-features
git diff --check
```

Expected: every command succeeds.

- [ ] **Step 3: Verify release binary behavior**

```bash
cargo build --release -p dotall-mcp
test -x target/release/dotall-mcp
```

Start it only through an MCP client; a direct launch should wait quietly for
protocol input and write nothing to stdout.

- [ ] **Step 4: Optional final commit checkpoint**

```bash
git add Cargo.toml Cargo.lock crates/dotall-mcp docs
git commit -m "feat: expose Dotall through a local MCP server"
```

## Self-review record

- Covers discoverable tool schemas, inspect/read/edit/version parity with CLI,
  structured agent recovery, stale-write protection, idempotent retries, official
  stdio transport, protocol-pollution tests, feature-gated formats, and installation.
- Core remains independent of MCP and all file operations flow through the same
  tested engine.
