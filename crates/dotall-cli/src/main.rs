use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use dotall_core::{DotallError, DotallStore, ObjectStatus};
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
    }
    Ok(())
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
