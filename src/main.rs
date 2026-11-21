use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cmdmesh::{config::Environment, inventory, read_json, recipe, write_private_json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cmdmesh",
    version,
    about = "Explicit inventory and reusable operations recipes"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Inventory {
        #[command(subcommand)]
        command: Inventory,
    },
    Drift {
        #[command(subcommand)]
        command: Drift,
    },
    Recipe {
        #[command(subcommand)]
        command: Recipe,
    },
    /// Read-only stdio MCP server; no recipe execution tool.
    Mcp,
}
#[derive(Subcommand)]
enum Inventory {
    Pull {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
enum Drift {
    Detect {
        #[arg(long)]
        desired: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
    },
}
#[derive(Subcommand)]
enum Recipe {
    List {
        #[arg(long)]
        catalog: PathBuf,
    },
    Inspect {
        manifest: PathBuf,
    },
    Plan {
        manifest: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long, value_enum)]
        operation: recipe::Operation,
        #[arg(long)]
        out: PathBuf,
    },
    Execute {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        approve_digest: String,
        /// Save bounded raw tool output in a new owner-only directory.
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    cmdmesh::mcp::install_cancellation_handlers();
    match cli.command {
        Command::Inventory {
            command: Inventory::Pull { provider, out },
        } => {
            let environment = load_environment(cli.config)?;
            let snapshot = inventory::pull(&environment, &provider)?;
            if let Some(out) = out {
                write_private_json(&out, &snapshot)?;
            } else {
                print_json(&snapshot)?;
            }
        }
        Command::Drift {
            command: Drift::Detect { desired, inventory },
        } => print_json(&inventory::drift_values(
            read_json(&desired)?,
            read_json(&inventory)?,
        )?)?,
        Command::Recipe { command } => match command {
            Recipe::List { catalog } => print_json(&recipe::list(&catalog)?)?,
            Recipe::Inspect { manifest } => print_json(&recipe::inspect(&manifest)?)?,
            Recipe::Plan {
                manifest,
                target,
                operation,
                out,
            } => {
                let environment = load_environment(cli.config)?;
                let plan = recipe::plan(&environment, &manifest, &target, operation)?;
                write_private_json(&out, &plan)?;
                print_json(&serde_json::json!({"digest":plan.digest,"out":out,"warning":plan.warning}))?;
            }
            Recipe::Execute {
                plan,
                approve_digest,
                log_dir,
            } => print_json(&recipe::execute_with_logs(
                &read_json(&plan)?,
                &approve_digest,
                log_dir.as_deref(),
            )?)?,
        },
        Command::Mcp => cmdmesh::mcp::serve(cli.config).await?,
    }
    Ok(())
}
fn load_environment(config: Option<PathBuf>) -> Result<Environment> {
    Environment::load(&config.context("--config is required for this operation")?)
}
fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
