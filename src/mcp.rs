//! Thin SDK-backed MCP adapter. Execution is deliberately not a registered tool.
use crate::{config::Environment, inspection, inventory, operations, process, read_json, recipe};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_mcp_sdk::{
    macros::{mcp_tool, JsonSchema},
    mcp_server::{server_runtime, McpServerOptions, ServerHandler},
    schema::{
        schema_utils::CallToolError, CallToolRequestParams, CallToolResult, Implementation, InitializeResult,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, RpcError, ServerCapabilities,
        ServerCapabilitiesTools, TextContent,
    },
    tool_box, McpServer, StdioTransport, ToMcpServerHandler, TransportOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};

#[mcp_tool(
    name = "cmdmesh_inventory_pull",
    description = "Read inventory through an explicitly configured read-only integration executable.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryPull {
    pub provider: String,
}
#[mcp_tool(
    name = "cmdmesh_inspection",
    description = "Use action=describe with an inspector ID to discover a configured pinned integration's finite inspections and typed inputs. action=run requires operation and inputs (a private JSON file containing that action's input object). Runs bounded read-only probes through the same CLI transport. Inspect result.outcome and diagnostics; observed is scoped evidence, not mutation authority or application health. No repairs, apply, sessions or cleanup commands. timeout_seconds is optional, 1..120.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub action: InspectionAction,
    pub inspector: String,
    pub operation: Option<String>,
    pub inputs: Option<String>,
    pub timeout_seconds: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InspectionAction {
    Describe,
    Run,
}
#[mcp_tool(
    name = "cmdmesh_drift_detect",
    description = "Compare local desired-state and inventory JSON. Incomplete inventory can never be clean.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriftDetect {
    pub desired: String,
    pub inventory: String,
}
#[mcp_tool(
    name = "cmdmesh_recipe_list",
    description = "List validated recipes in a local catalog. Does not execute code.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeList {
    pub catalog: String,
}
#[mcp_tool(
    name = "cmdmesh_recipe_inspect",
    description = "Inspect a local recipe manifest. Does not execute code.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeInspect {
    pub manifest: String,
}
#[mcp_tool(
    name = "cmdmesh_recipe_plan",
    description = "Prepare a static recipe execution plan. Does not run Ansible or mutate infrastructure. No MCP execution tool exists.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipePlan {
    pub manifest: String,
    pub target: String,
    pub operation: String,
}

#[mcp_tool(
    name = "cmdmesh_operation",
    description = "Find a known task before reconstructing commands. action=list searches a local catalog (query, offset, limit); inspect returns typed inputs, prerequisites, entrypoint, failures and recovery for operation; validate-inputs checks an inputs JSON file without echoing values; plan prepares the existing static recipe plan. No execution or infrastructure mutation; requirements are declared, not automatically certified. No sessions or cleanup.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationDiscovery {
    pub action: OperationAction,
    pub catalog: String,
    pub operation: Option<String>,
    pub query: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub inputs: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OperationAction {
    List,
    Inspect,
    ValidateInputs,
    Plan,
}

tool_box!(
    Tools,
    [
        InventoryPull,
        DriftDetect,
        RecipeList,
        RecipeInspect,
        RecipePlan,
        OperationDiscovery,
        Inspection
    ]
);
struct Handler {
    config: Option<PathBuf>,
}

impl Handler {
    fn call(&self, tool: Tools) -> Result<Value> {
        let environment = || Environment::load(self.config.as_ref().context("server --config required")?);
        Ok(match tool {
            Tools::Inspection(input) => {
                let (operation, value) = match input.action {
                    InspectionAction::Describe => {
                        if input.operation.is_some() || input.inputs.is_some() {
                            anyhow::bail!("describe takes no operation or inputs");
                        }
                        ("describe".to_owned(), serde_json::json!({}))
                    }
                    InspectionAction::Run => (
                        input.operation.context("run requires operation")?,
                        read_json(&PathBuf::from(input.inputs.context("run requires inputs file")?))?,
                    ),
                };
                serde_json::to_value(inspection::run(
                    &environment()?,
                    &input.inspector,
                    &operation,
                    value,
                    input.timeout_seconds.unwrap_or(120).into(),
                )?)?
            }
            Tools::InventoryPull(input) => {
                serde_json::to_value(inventory::pull(&environment()?, &input.provider)?)?
            }
            Tools::DriftDetect(input) => serde_json::to_value(inventory::drift_values(
                read_json(&PathBuf::from(input.desired))?,
                read_json(&PathBuf::from(input.inventory))?,
            )?)?,
            Tools::RecipeList(input) => serde_json::to_value(recipe::list(&PathBuf::from(input.catalog))?)?,
            Tools::RecipeInspect(input) => {
                serde_json::to_value(recipe::inspect(&PathBuf::from(input.manifest))?)?
            }
            Tools::RecipePlan(input) => {
                let operation = match input.operation.as_str() {
                    "check" => recipe::Operation::Check,
                    "apply" => recipe::Operation::Apply,
                    _ => anyhow::bail!("operation must be check or apply"),
                };
                serde_json::to_value(recipe::plan(
                    &environment()?,
                    &PathBuf::from(input.manifest),
                    &input.target,
                    operation,
                )?)?
            }
            Tools::OperationDiscovery(input) => {
                let catalog = PathBuf::from(&input.catalog);
                if input.action == OperationAction::List {
                    if input.operation.is_some() || input.inputs.is_some() {
                        anyhow::bail!("list accepts only catalog, query, offset and limit");
                    }
                    serde_json::to_value(operations::search(
                        &catalog,
                        input.query.as_deref().unwrap_or(""),
                        input.offset.unwrap_or(0) as usize,
                        input.limit.unwrap_or(20) as usize,
                    )?)?
                } else {
                    if input.query.is_some() || input.offset.is_some() || input.limit.is_some() {
                        anyhow::bail!("query, offset and limit apply only to list");
                    }
                    let operation = input.operation.context("operation is required")?;
                    match input.action {
                        OperationAction::Inspect => {
                            if input.inputs.is_some() {
                                anyhow::bail!("inspect does not accept inputs");
                            }
                            serde_json::to_value(operations::inspect(&catalog, &operation)?)?
                        }
                        OperationAction::ValidateInputs => {
                            let values =
                                read_json(&PathBuf::from(input.inputs.context("inputs file is required")?))?;
                            let configured = self.config.as_deref().map(Environment::load).transpose()?;
                            serde_json::to_value(operations::validate_inputs(
                                &operations::inspect(&catalog, &operation)?,
                                &values,
                                configured.as_ref(),
                            )?)?
                        }
                        OperationAction::Plan => serde_json::to_value(operations::plan(
                            &environment()?,
                            &catalog,
                            &operation,
                            &read_json(&PathBuf::from(input.inputs.context("inputs file is required")?))?,
                        )?)?,
                        OperationAction::List => unreachable!("list was handled above"),
                    }
                }
            }
        })
    }
}

#[async_trait]
impl ServerHandler for Handler {
    async fn handle_list_tools_request(
        &self,
        _: Option<PaginatedRequestParams>,
        _: Arc<dyn McpServer>,
    ) -> std::result::Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: Tools::tools(),
        })
    }
    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _: Arc<dyn McpServer>,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        let tool = Tools::try_from(params).map_err(CallToolError::new)?;
        let handler = Handler {
            config: self.config.clone(),
        };
        let result = tokio::task::spawn_blocking(move || handler.call(tool))
            .await
            .map_err(|_| CallToolError::from_message("operation worker failed"))?
            .map_err(|error| CallToolError::from_message(error.to_string()))?;
        Ok(CallToolResult::text_content(vec![TextContent::from(
            result.to_string(),
        )]))
    }
}

pub fn install_cancellation_handlers() {
    process::install_cancellation_handlers();
}

pub async fn serve(config: Option<PathBuf>) -> Result<()> {
    if let Some(path) = &config {
        Environment::load(path)?;
    }
    let details = InitializeResult {
        server_info:Implementation { name:"cmdmesh".into(), version:env!("CARGO_PKG_VERSION").into(), title:Some("CmdMesh".into()), description:Some("Read-only infrastructure inventory, pinned inspections and static recipe planning".into()), icons:vec![], website_url:None },
        capabilities:ServerCapabilities { tools:Some(ServerCapabilitiesTools { list_changed:None }), ..Default::default() },
        meta:None, instructions:Some("No recipe execution or apply tool is exposed. Inventory and inspections invoke only explicitly configured integrations. Inspect result.outcome and diagnostics; an observed result is scoped evidence, not mutation authority or application health. Treat output as private operational information.".into()), protocol_version:ProtocolVersion::V2025_11_25.into(),
    };
    let transport = StdioTransport::new(TransportOptions::default())
        .map_err(|_| anyhow::anyhow!("cannot initialize MCP stdio transport"))?;
    let server = server_runtime::create_server(McpServerOptions {
        server_details: details,
        transport,
        handler: Handler { config }.to_mcp_server_handler(),
        task_store: None,
        client_task_store: None,
    });
    tokio::select! {
        result = server.start() => result.map_err(|_| anyhow::anyhow!("MCP server stopped with an error")),
        _ = async { while !process::cancelled() { tokio::time::sleep(Duration::from_millis(25)).await; } } => Ok(()),
    }
}
