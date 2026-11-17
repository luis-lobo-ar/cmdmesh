//! Thin SDK-backed MCP adapter. Execution is deliberately not a registered tool.
use crate::{config::Environment, inventory, process, read_json, recipe};
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

tool_box!(
    Tools,
    [InventoryPull, DriftDetect, RecipeList, RecipeInspect, RecipePlan]
);
struct Handler {
    config: Option<PathBuf>,
}

impl Handler {
    fn call(&self, tool: Tools) -> Result<Value> {
        let environment = || Environment::load(self.config.as_ref().context("server --config required")?);
        Ok(match tool {
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
        server_info:Implementation { name:"cmdmesh".into(), version:env!("CARGO_PKG_VERSION").into(), title:Some("CmdMesh".into()), description:Some("Read-only infrastructure inventory and static recipe planning".into()), icons:vec![], website_url:None },
        capabilities:ServerCapabilities { tools:Some(ServerCapabilitiesTools { list_changed:None }), ..Default::default() },
        meta:None, instructions:Some("No recipe execution or apply tool is exposed. Inventory invokes only configured integrations. Treat output as private operational information.".into()), protocol_version:ProtocolVersion::V2025_11_25.into(),
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
