use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cmdmesh::{
    config::Environment, inspection, inventory, operations, procedure, read_json, recipe, write_private_json,
};
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
    /// Plan and execute the fixed verified owned-memory procedure.
    Procedure {
        #[command(subcommand)]
        command: Procedure,
    },
    /// Invoke a configured pinned inspection integration. No apply action.
    Inspection {
        #[command(subcommand)]
        command: Inspection,
    },
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
    /// Discover a known operation and prepare its existing recipe plan.
    Operation {
        #[command(subcommand)]
        command: Operations,
    },
    /// Inspection and planning MCP server; no execution tool.
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
enum Procedure {
    Describe,
    Plan {
        #[arg(long)]
        inspector: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        state_directory: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Execute {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        approve_digest: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Status {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
enum Inspection {
    Describe {
        #[arg(long)]
        inspector: String,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Run {
        #[arg(long)]
        inspector: String,
        #[arg(long)]
        operation: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
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

#[derive(Subcommand)]
enum Operations {
    List {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Inspect {
        operation: String,
        #[arg(long)]
        catalog: PathBuf,
    },
    ValidateInputs {
        operation: String,
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        inputs: PathBuf,
    },
    Plan {
        operation: String,
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        inputs: PathBuf,
        #[arg(long)]
        out: PathBuf,
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
        Command::Procedure { command } => match command {
            Procedure::Describe => print_json(&procedure::describe())?,
            Procedure::Plan {
                inspector,
                input,
                state_directory,
                out,
            } => {
                let plan = procedure::plan(
                    &load_environment(cli.config)?,
                    &inspector,
                    read_json(&input)?,
                    &state_directory,
                )?;
                write_private_json(&out, &plan)?;
                print_json(
                    &serde_json::json!({"digest":plan.digest,"out":out,"retained_plan":plan.context_directory.join("plan.json"),"warning":"Review the exact subject, memory, permissions and bindings. A consumed plan cannot be replayed; status is historical."}),
                )?;
            }
            Procedure::Execute {
                plan,
                approve_digest,
                out,
            } => {
                let result = procedure::execute(&read_json(&plan)?, &approve_digest)?;
                if let Some(out) = out {
                    write_private_json(&out, &result)?;
                } else {
                    print_json(&result)?;
                }
                if !matches!(
                    result.outcome,
                    procedure::Outcome::Verified | procedure::Outcome::Checked
                ) {
                    anyhow::bail!("procedure did not verify; inspect retained result and phase diagnostics");
                }
            }
            Procedure::Status { plan, out } => {
                let result = procedure::status(&read_json(&plan)?)?;
                if let Some(out) = out {
                    write_private_json(&out, &result)?;
                } else {
                    print_json(&result)?;
                }
            }
        },
        Command::Inspection { command } => {
            let (inspector, operation, input, timeout_seconds, out) = match command {
                Inspection::Describe {
                    inspector,
                    timeout_seconds,
                    out,
                } => (
                    inspector,
                    "describe".to_owned(),
                    serde_json::json!({}),
                    timeout_seconds,
                    out,
                ),
                Inspection::Run {
                    inspector,
                    operation,
                    input,
                    timeout_seconds,
                    out,
                } => (inspector, operation, read_json(&input)?, timeout_seconds, out),
            };
            let receipt = inspection::run(
                &load_environment(cli.config)?,
                &inspector,
                &operation,
                input,
                timeout_seconds,
            )?;
            if let Some(out) = out {
                write_private_json(&out, &receipt)?;
            } else {
                print_json(&receipt)?;
            }
            if receipt.result.outcome != inspection::Outcome::Observed {
                anyhow::bail!("inspection did not produce an observed outcome; see receipt diagnostics");
            }
        }
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
        Command::Operation { command } => match command {
            Operations::List {
                catalog,
                query,
                offset,
                limit,
            } => {
                print_json(&operations::search(&catalog, &query, offset, limit)?)?;
            }
            Operations::Inspect { operation, catalog } => {
                print_json(&operations::inspect(&catalog, &operation)?)?;
            }
            Operations::ValidateInputs {
                operation,
                catalog,
                inputs,
            } => {
                let environment = cli.config.as_deref().map(Environment::load).transpose()?;
                let report = operations::validate_inputs(
                    &operations::inspect(&catalog, &operation)?,
                    &read_json(&inputs)?,
                    environment.as_ref(),
                )?;
                print_json(&report)?;
                if !report.valid {
                    anyhow::bail!("operation inputs are invalid; see diagnostic codes");
                }
            }
            Operations::Plan {
                operation,
                catalog,
                inputs,
                out,
            } => {
                let environment = load_environment(cli.config)?;
                let plan = operations::prepare(&environment, &catalog, &operation, &read_json(&inputs)?)?;
                write_private_json(&out, &plan)?;
                print_json(&serde_json::json!({"digest":plan.digest(),"out":out,"warning":plan.warning()}))?;
            }
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
