use std::path::{Path, PathBuf};
use std::process::ExitCode;
#[cfg(feature = "xlsx")]
use std::sync::Arc;

use clap::{Parser, Subcommand};
use dotall_core::registry::{FormatRegistry, ReadRequest, ReadSelector};
use dotall_core::{DotallError, DotallStore, Engine, ObjectStatus};
use serde::Serialize;

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
        #[arg(long, conflicts_with_all = ["sheet", "ast_range"])]
        range: Option<String>,
        #[arg(long, conflicts_with_all = ["range", "ast_range"])]
        sheet: Option<String>,
        #[arg(long, conflicts_with_all = ["range", "sheet"])]
        ast_range: Option<String>,
        #[arg(long, default_value_t = 2_000)]
        max_tokens: usize,
        #[arg(long)]
        continuation: Option<String>,
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
            let store = DotallStore::open(path)?;
            let objects = store.status()?;
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
            require_xlsx_support()?;
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
            max_tokens,
            continuation,
        } => {
            require_xlsx_support()?;
            let (store, relative) = open_file_workspace(path)?;
            let mut engine = Engine::new(store, default_registry());
            let request = ReadRequest {
                selector: selector_from_args(range, sheet, ast_range),
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
    }
    Ok(())
}

fn selector_from_args(
    range: &Option<String>,
    sheet: &Option<String>,
    ast_range: &Option<String>,
) -> Option<ReadSelector> {
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

#[cfg(feature = "xlsx")]
fn default_registry() -> FormatRegistry {
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(dotall_xlsx::XlsxFormat));
    registry
}

#[cfg(not(feature = "xlsx"))]
fn default_registry() -> FormatRegistry {
    FormatRegistry::default()
}

#[cfg(feature = "xlsx")]
fn require_xlsx_support() -> dotall_core::Result<()> {
    Ok(())
}

#[cfg(not(feature = "xlsx"))]
fn require_xlsx_support() -> dotall_core::Result<()> {
    Err(DotallError::Format {
        format_id: "dotall-cli".into(),
        path: PathBuf::from("<build>"),
        message: "XLSX support is disabled; rebuild with `--features xlsx`.".into(),
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
            "Rebuild dotall-cli with --features xlsx."
        }
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
