use std::path::{Path, PathBuf};
use std::process::ExitCode;
#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
use std::sync::Arc;

use clap::{Parser, Subcommand};
use dotall_core::registry::{FormatRegistry, ReadRequest, ReadSelector, SemanticOperation};
use dotall_core::{
    Actor, ActorKind, AppliedEdit, DotallError, DotallStore, EditRequest, Engine, HistoryRecord,
    HistorySummary, ObjectStatus, SearchRequest, StagedEdit, search_store, viz_snapshot,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "dotall", about = "Agent-native file access and editing")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Inspect {
        path: PathBuf,
    },
    Read {
        path: PathBuf,
        #[arg(long, conflicts_with_all = ["sheet", "ast_range", "selector_kind"])]
        range: Option<String>,
        #[arg(long, conflicts_with_all = ["range", "ast_range", "selector_kind"])]
        sheet: Option<String>,
        #[arg(long, conflicts_with_all = ["range", "sheet", "selector_kind"])]
        ast_range: Option<String>,
        #[arg(long, conflicts_with_all = ["range", "sheet", "ast_range"])]
        selector_kind: Option<String>,
        #[arg(
            long,
            conflicts_with_all = ["range", "sheet", "ast_range"],
            requires = "selector_kind"
        )]
        selector: Option<String>,
        #[arg(long, default_value_t = 2_000)]
        max_tokens: usize,
        #[arg(long)]
        continuation: Option<String>,
    },
    Deps {
        path: PathBuf,
        #[arg(long)]
        cell: String,
        #[arg(long)]
        dependents: bool,
    },
    Edit {
        path: PathBuf,
        #[arg(long, conflicts_with = "ops_json")]
        op: Option<String>,
        #[arg(
            long,
            conflicts_with_all = ["ops_json", "sheet", "address", "value", "formula"]
        )]
        payload_json: Option<String>,
        #[arg(long)]
        sheet: Option<String>,
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        value: Option<String>,
        #[arg(long)]
        formula: Option<String>,
        #[arg(long, conflicts_with = "op")]
        ops_json: Option<String>,
        #[arg(long)]
        tx: Option<Uuid>,
        #[arg(long)]
        expected_hash: Option<String>,
    },
    Apply {
        path: PathBuf,
        #[arg(long, conflicts_with = "all")]
        tx: Option<Uuid>,
        #[arg(long, conflicts_with = "tx")]
        all: bool,
    },
    Discard {
        path: PathBuf,
        #[arg(long)]
        tx: Uuid,
    },
    Staged {
        path: PathBuf,
    },
    History {
        path: PathBuf,
    },
    Diff {
        path: PathBuf,
        #[arg(long)]
        version: u64,
    },
    Revert {
        path: PathBuf,
        #[arg(long)]
        version: u64,
        #[arg(long)]
        tx: Option<Uuid>,
    },
    Search {
        query: String,
        #[arg(long)]
        glob: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Viz {
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Serialize)]
struct InitOutput {
    workspace: PathBuf,
    initialized: bool,
}

#[derive(Debug, Serialize)]
struct StatusOutput {
    workspace: PathBuf,
    tracked_count: usize,
    objects: Vec<ObjectStatus>,
}

#[derive(Debug, Serialize)]
struct HistoryOutput {
    entries: Vec<HistorySummary>,
}

#[derive(Debug, Serialize)]
struct StagedOutput {
    edits: Vec<StagedEdit>,
}

#[derive(Debug, Serialize)]
struct ApplyAllOutput {
    applied: Vec<AppliedEdit>,
}

struct EditOptions<'a> {
    op: Option<&'a str>,
    payload_json: Option<&'a str>,
    sheet: Option<&'a str>,
    address: Option<&'a str>,
    value: Option<&'a str>,
    formula: Option<&'a str>,
    ops_json: Option<&'a str>,
    tx: Option<Uuid>,
    expected_hash: Option<&'a str>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            render_error(&error, cli.json);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> dotall_core::Result<()> {
    match &cli.command {
        Command::Init { path } => {
            let already_initialized = path.join(".all/manifest.json").is_file();
            let store = DotallStore::init(path)?;
            let output = InitOutput {
                workspace: store.workspace().root().to_path_buf(),
                initialized: !already_initialized,
            };
            if cli.json {
                print_json(&output);
            } else if output.initialized {
                println!("Initialized Dotall in {}", output.workspace.display());
            } else {
                println!(
                    "Dotall already initialized in {}",
                    output.workspace.display()
                );
            }
        }
        Command::Status { path } => {
            let mut store = DotallStore::open(path)?;
            let objects = store.refresh_fresh_fingerprints()?;
            let output = StatusOutput {
                workspace: store.workspace().root().to_path_buf(),
                tracked_count: objects.len(),
                objects,
            };
            if cli.json {
                print_json(&output);
            } else {
                println!(
                    "{} tracked file(s) in {}",
                    output.tracked_count,
                    output.workspace.display()
                );
                for object in output.objects {
                    println!("{:?}\t{}\t{}", object.state, object.format_id, object.path);
                }
            }
        }
        Command::Inspect { path } => {
            require_format_support()?;
            let (store, relative) = open_file_workspace(path)?;
            let mut engine = Engine::new(store, default_registry());
            let output = engine.inspect(&relative)?;
            print_json(&output);
        }
        Command::Read {
            path,
            range,
            sheet,
            ast_range,
            selector_kind,
            selector,
            max_tokens,
            continuation,
        } => {
            require_format_support()?;
            let (store, relative) = open_file_workspace(path)?;
            let mut engine = Engine::new(store, default_registry());
            let request = ReadRequest {
                selector: selector_from_args(range, sheet, ast_range, selector_kind, selector),
                max_tokens: *max_tokens,
                continuation: continuation.clone(),
            };
            let output = engine.read(&relative, &request)?;
            if cli.json {
                print_json(&output);
            } else {
                print!("{}", output.response.content);
                if !output.response.content.ends_with('\n') {
                    println!();
                }
                if output.response.truncated
                    && let Some(continuation) = output.response.continuation
                {
                    println!("continuation: {continuation}");
                }
            }
        }
        Command::Deps {
            path,
            cell,
            dependents,
        } => run_deps(path, cell, *dependents)?,
        Command::Edit {
            path,
            op,
            payload_json,
            sheet,
            address,
            value,
            formula,
            ops_json,
            tx,
            expected_hash,
        } => run_edit(
            cli,
            path,
            &EditOptions {
                op: op.as_deref(),
                payload_json: payload_json.as_deref(),
                sheet: sheet.as_deref(),
                address: address.as_deref(),
                value: value.as_deref(),
                formula: formula.as_deref(),
                ops_json: ops_json.as_deref(),
                tx: *tx,
                expected_hash: expected_hash.as_deref(),
            },
        )?,
        Command::Apply { path, tx, all } => run_apply(cli, path, *tx, *all)?,
        Command::Discard { path, tx } => run_discard(cli, path, *tx)?,
        Command::Staged { path } => run_staged(cli, path)?,
        Command::History { path } => run_history(cli, path)?,
        Command::Diff { path, version } => run_diff(cli, path, *version)?,
        Command::Revert { path, version, tx } => run_revert(cli, path, *version, *tx)?,
        Command::Search { query, glob, path } => run_search(cli, query, glob, path)?,
        Command::Viz {
            port,
            no_open,
            path,
        } => run_viz(path, *port, *no_open)?,
    }
    Ok(())
}

fn run_search(
    cli: &Cli,
    query: &str,
    glob: &Option<String>,
    path: &Path,
) -> dotall_core::Result<()> {
    let store = DotallStore::open(path)?;
    let results = search_store(
        &store,
        &SearchRequest {
            query: query.to_string(),
            glob: glob.clone(),
        },
    )?;
    if cli.json {
        print_json(&results);
    } else {
        for hit in &results.hits {
            println!(
                "{}\t{}\t{}\t{}",
                hit.path,
                hit.selector_kind.as_deref().unwrap_or(""),
                hit.selector.as_deref().unwrap_or(""),
                hit.snippet
            );
        }
    }
    Ok(())
}

fn run_viz(path: &Path, port: u16, no_open: bool) -> dotall_core::Result<()> {
    let store = DotallStore::open(path)?;
    let addr = format!("127.0.0.1:{port}");
    let server = tiny_http::Server::http(&addr).map_err(|error| DotallError::InvalidArgument {
        reason: format!("failed to bind viz server on {addr}: {error}"),
    })?;
    let bound = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| DotallError::InvalidArgument {
            reason: "viz server did not bind an IP address".into(),
        })?;
    let url = format!("http://127.0.0.1:{}/", bound.port());
    println!("{url}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    if !no_open {
        let _ = webbrowser::open(&url);
    }

    for request in server.incoming_requests() {
        let snapshot = match viz_snapshot(&store) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let body = error.to_string();
                let response = tiny_http::Response::from_data(body.into_bytes())
                    .with_status_code(500)
                    .with_header(content_type("text/plain; charset=utf-8"));
                let _ = request.respond(response);
                continue;
            }
        };

        let url_path = request.url().split('?').next().unwrap_or("/");
        let response = match (request.method(), url_path) {
            (&tiny_http::Method::Get, "/") | (&tiny_http::Method::Get, "/index.html") => {
                let body = include_str!("viz/index.html");
                tiny_http::Response::from_data(body.as_bytes())
                    .with_header(content_type("text/html; charset=utf-8"))
            }
            (&tiny_http::Method::Get, "/api/tree") => {
                let body = serde_json::json!({
                    "tree": snapshot.tree,
                    "history": snapshot.history,
                });
                json_response(&body)
            }
            (&tiny_http::Method::Get, "/api/metrics") => json_response(&snapshot.metrics),
            _ => tiny_http::Response::from_data(b"Not Found".as_slice())
                .with_status_code(404)
                .with_header(content_type("text/plain; charset=utf-8")),
        };
        let _ = request.respond(response);
    }
    Ok(())
}

fn content_type(value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(&b"Content-Type"[..], value.as_bytes()).unwrap_or_else(|_| {
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/octet-stream"[..])
            .expect("fallback content-type header")
    })
}

fn json_response<T: Serialize>(value: &T) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|error| {
        format!(r#"{{"error":"failed to serialize: {error}"}}"#).into_bytes()
    });
    tiny_http::Response::from_data(body)
        .with_header(content_type("application/json; charset=utf-8"))
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_edit(cli: &Cli, path: &Path, options: &EditOptions<'_>) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let mut engine = Engine::new(store, default_registry());
    let operations = build_operations(
        options.op,
        options.payload_json,
        options.sheet,
        options.address,
        options.value,
        options.formula,
        options.ops_json,
    )?;
    let expected_source_hash = match options.expected_hash {
        Some(hash) => hash.to_string(),
        None => engine.inspect(&relative)?.source_hash,
    };
    let staged = engine.edit(
        &relative,
        &EditRequest {
            transaction_id: options.tx.unwrap_or_else(Uuid::new_v4),
            expected_source_hash,
            actor: cli_actor(),
            operations,
        },
    )?;
    render_staged(cli, &staged);
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_edit(_cli: &Cli, _path: &Path, _options: &EditOptions<'_>) -> dotall_core::Result<()> {
    require_format_support()
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_apply(cli: &Cli, path: &Path, tx: Option<Uuid>, all: bool) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let mut engine = Engine::new(store, default_registry());

    if all {
        let applied = engine.apply_all(&relative)?;
        if cli.json {
            print_json(&ApplyAllOutput { applied });
        } else {
            for result in applied {
                print_applied(&result);
            }
        }
        return Ok(());
    }

    let tx_id = tx.ok_or_else(|| DotallError::InvalidSourcePath {
        path: path.to_path_buf(),
        reason: "apply requires --tx or --all".into(),
    })?;
    let applied = engine.apply(&relative, tx_id)?;
    render_applied(cli, &applied);
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_apply(_cli: &Cli, _path: &Path, _tx: Option<Uuid>, _all: bool) -> dotall_core::Result<()> {
    require_format_support()
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_discard(cli: &Cli, path: &Path, tx: Uuid) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let engine = Engine::new(store, default_registry());
    engine.discard(&relative, tx)?;
    if cli.json {
        print_json(&serde_json::json!({ "discarded": tx.to_string() }));
    } else {
        println!("discarded staged edit {tx}");
    }
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_discard(_cli: &Cli, _path: &Path, _tx: Uuid) -> dotall_core::Result<()> {
    require_format_support()
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_staged(cli: &Cli, path: &Path) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let engine = Engine::new(store, default_registry());
    let edits = engine.staged(&relative)?;
    if cli.json {
        print_json(&StagedOutput { edits });
    } else if edits.is_empty() {
        println!("no staged edits");
    } else {
        for edit in edits {
            println!(
                "{}  {} op(s)  expected_hash={}",
                edit.tx_id,
                edit.preview.operations.len(),
                edit.expected_source_hash
            );
        }
    }
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_staged(_cli: &Cli, _path: &Path) -> dotall_core::Result<()> {
    require_format_support()
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_history(cli: &Cli, path: &Path) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let engine = Engine::new(store, default_registry());
    let entries = engine.history(&relative)?;
    if cli.json {
        print_json(&HistoryOutput { entries });
    } else if entries.is_empty() {
        println!("no history");
    } else {
        for entry in entries {
            println!(
                "v{}  {}  {}  {} op(s)",
                entry.version, entry.timestamp, entry.summary, entry.op_count
            );
        }
    }
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_history(_cli: &Cli, _path: &Path) -> dotall_core::Result<()> {
    require_format_support()
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_diff(cli: &Cli, path: &Path, version: u64) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let engine = Engine::new(store, default_registry());
    let record = engine.diff(&relative, version)?;
    render_diff(cli, &record);
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_diff(_cli: &Cli, _path: &Path, _version: u64) -> dotall_core::Result<()> {
    require_format_support()
}

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn run_revert(cli: &Cli, path: &Path, version: u64, tx: Option<Uuid>) -> dotall_core::Result<()> {
    require_format_support()?;
    let (store, relative) = open_file_workspace(path)?;
    let mut engine = Engine::new(store, default_registry());
    let staged = engine.revert(&relative, version, tx.unwrap_or_else(Uuid::new_v4))?;
    render_staged(cli, &staged);
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn run_revert(
    _cli: &Cli,
    _path: &Path,
    _version: u64,
    _tx: Option<Uuid>,
) -> dotall_core::Result<()> {
    require_format_support()
}

fn build_operations(
    op: Option<&str>,
    payload_json: Option<&str>,
    sheet: Option<&str>,
    address: Option<&str>,
    value: Option<&str>,
    formula: Option<&str>,
    ops_json: Option<&str>,
) -> dotall_core::Result<Vec<SemanticOperation>> {
    if let Some(raw) = ops_json {
        return serde_json::from_str(raw).map_err(|source| DotallError::Serialization {
            context: "edit --ops-json".into(),
            source,
        });
    }

    let op = op.ok_or_else(|| DotallError::InvalidSourcePath {
        path: PathBuf::from("<edit>"),
        reason: "edit requires --op or --ops-json".into(),
    })?;
    if let Some(raw) = payload_json {
        let payload = serde_json::from_str(raw).map_err(|source| DotallError::Serialization {
            context: "edit --payload-json".into(),
            source,
        })?;
        return Ok(vec![SemanticOperation {
            kind: op.into(),
            payload,
        }]);
    }
    let sheet = sheet.ok_or_else(|| DotallError::InvalidSourcePath {
        path: PathBuf::from("<edit>"),
        reason: "edit requires --sheet".into(),
    })?;
    let address = address.ok_or_else(|| DotallError::InvalidSourcePath {
        path: PathBuf::from("<edit>"),
        reason: "edit requires --address".into(),
    })?;

    let payload = match op {
        "set_cell_value" => {
            let value = value.ok_or_else(|| DotallError::InvalidSourcePath {
                path: PathBuf::from("<edit>"),
                reason: "set_cell_value requires --value".into(),
            })?;
            serde_json::json!({
                "sheet": sheet,
                "address": address,
                "value": parse_edit_value(value)?,
            })
        }
        "set_cell_formula" => {
            let formula = formula.ok_or_else(|| DotallError::InvalidSourcePath {
                path: PathBuf::from("<edit>"),
                reason: "set_cell_formula requires --formula".into(),
            })?;
            serde_json::json!({
                "sheet": sheet,
                "address": address,
                "formula": formula,
            })
        }
        other => {
            return Err(DotallError::UnsupportedCapability {
                format_id: "cli".into(),
                capability: format!("edit --op {other}"),
                available: vec![
                    "set_cell_value".into(),
                    "set_cell_formula".into(),
                    "--ops-json".into(),
                ],
            });
        }
    };

    Ok(vec![SemanticOperation {
        kind: op.into(),
        payload,
    }])
}

fn parse_edit_value(raw: &str) -> dotall_core::Result<serde_json::Value> {
    if let Ok(number) = raw.parse::<f64>() {
        return Ok(serde_json::json!(number));
    }
    if raw.eq_ignore_ascii_case("true") {
        return Ok(serde_json::json!(true));
    }
    if raw.eq_ignore_ascii_case("false") {
        return Ok(serde_json::json!(false));
    }
    if raw.eq_ignore_ascii_case("blank") {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::Value::String(raw.to_string()))
}

fn cli_actor() -> Actor {
    Actor {
        kind: ActorKind::Cli,
        id: Some("dotall".into()),
    }
}

fn render_staged(cli: &Cli, staged: &StagedEdit) {
    if cli.json {
        print_json(staged);
    } else {
        println!("staged edit {}", staged.tx_id);
        for change in &staged.preview.semantic_diff {
            println!(
                "  {} {} -> {}",
                change.target,
                change.before.as_deref().unwrap_or("<empty>"),
                change.after.as_deref().unwrap_or("<empty>")
            );
        }
    }
}

fn render_applied(cli: &Cli, applied: &AppliedEdit) {
    if cli.json {
        print_json(applied);
    } else {
        print_applied(applied);
    }
}

fn print_applied(applied: &AppliedEdit) {
    println!("applied version {} (tx {})", applied.version, applied.tx_id);
    println!(
        "  {} -> {}",
        applied.before_source_hash, applied.after_source_hash
    );
    if let Some(revert_of) = applied.revert_of {
        println!("  revert_of v{revert_of}");
    }
}

fn render_diff(cli: &Cli, record: &HistoryRecord) {
    if cli.json {
        print_json(record);
    } else {
        println!("version {} ({})", record.version, record.timestamp);
        for change in &record.semantic_diff {
            println!(
                "  {} {} -> {}",
                change.target,
                change.before.as_deref().unwrap_or("<empty>"),
                change.after.as_deref().unwrap_or("<empty>")
            );
        }
    }
}

fn selector_from_args(
    range: &Option<String>,
    sheet: &Option<String>,
    ast_range: &Option<String>,
    selector_kind: &Option<String>,
    selector: &Option<String>,
) -> Option<ReadSelector> {
    if let Some(kind) = selector_kind {
        return Some(ReadSelector {
            kind: kind.clone(),
            value: selector.clone().unwrap_or_default(),
        });
    }
    let (kind, value) = match (range, sheet, ast_range) {
        (Some(value), None, None) => ("range", value),
        (None, Some(value), None) => ("sheet", value),
        (None, None, Some(value)) => ("ast_range", value),
        (None, None, None) => return None,
        _ => unreachable!("clap rejects conflicting read selectors"),
    };
    Some(ReadSelector {
        kind: kind.into(),
        value: value.clone(),
    })
}

#[cfg(feature = "xlsx")]
fn run_deps(path: &Path, cell: &str, dependents: bool) -> dotall_core::Result<()> {
    let (store, relative) = open_file_workspace(path)?;
    let mut engine = Engine::new(store, default_registry());
    let output = engine.deps(&relative, cell, dependents)?;
    print_json(&output);
    Ok(())
}

#[cfg(not(feature = "xlsx"))]
fn run_deps(_path: &Path, _cell: &str, _dependents: bool) -> dotall_core::Result<()> {
    Err(DotallError::Format {
        format_id: "dotall-cli".into(),
        path: PathBuf::from("<build>"),
        message: "XLSX support is disabled; rebuild with `--features xlsx`.".into(),
    })
}

fn open_file_workspace(path: &Path) -> dotall_core::Result<(DotallStore, String)> {
    let store = DotallStore::open(path)?;
    let source = path
        .canonicalize()
        .map_err(|_| DotallError::InvalidSourcePath {
            path: path.to_path_buf(),
            reason: "source path must point to an existing file".into(),
        })?;
    let relative = source
        .strip_prefix(store.workspace().root())
        .map_err(|_| DotallError::InvalidSourcePath {
            path: source.clone(),
            reason: "source must be inside the initialized workspace".into(),
        })?
        .to_str()
        .ok_or_else(|| DotallError::InvalidSourcePath {
            path: source.clone(),
            reason: "source path must be valid UTF-8".into(),
        })?
        .replace('\\', "/");
    Ok((store, relative))
}

fn default_registry() -> FormatRegistry {
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

#[cfg(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf"))]
fn require_format_support() -> dotall_core::Result<()> {
    Ok(())
}

#[cfg(not(any(feature = "xlsx", feature = "pptx", feature = "docx", feature = "pdf")))]
fn require_format_support() -> dotall_core::Result<()> {
    Err(DotallError::Format {
        format_id: "dotall-cli".into(),
        path: PathBuf::from("<build>"),
        message: "no format handlers are enabled; rebuild with `--features xlsx`, `--features pptx`, `--features docx`, and/or `--features pdf`."
            .into(),
    })
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            let fallback = serde_json::json!({
                "error": format!("failed to serialize CLI output: {error}"),
            });
            eprintln!("{fallback}");
        }
    }
}

fn render_error(error: &DotallError, json: bool) {
    let next_action = match error {
        DotallError::WorkspaceNotInitialized(_) => "Run `dotall init <workspace>` first.",
        DotallError::Format { format_id, .. } if format_id == "dotall-cli" => {
            "Rebuild with --features xlsx, pptx, docx, and/or pdf."
        }
        DotallError::InvalidArgument { .. } => "Fix the argument and retry.",
        _ => "Inspect the path and retry the operation.",
    };
    if json {
        let value = serde_json::json!({
            "error": error.to_string(),
            "retryable": false,
            "next_action": next_action,
        });
        eprintln!("{value}");
    } else {
        eprintln!("error: {error}");
        eprintln!("next: {next_action}");
    }
}
