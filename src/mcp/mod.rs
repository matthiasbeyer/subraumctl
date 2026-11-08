//! Optional stdio frontend. Device tools are registered explicitly here, never
//! inferred from the CLI command tree or exposed through a generic CLI runner.

mod backend;
pub mod config;
mod snapshot;
mod tools;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use eyre::{Result, WrapErr, ensure};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};

use crate::cli::{GlobalArgs, McpArgs};
use config::{Access, Config, Group};

type ToolFuture = Pin<Box<dyn Future<Output = Result<CallToolResponse, ErrorData>> + Send>>;
type Handler =
    Box<dyn Fn(CallToolRequestParams, RequestContext<RoleServer>) -> ToolFuture + Send + Sync>;

struct RegisteredTool {
    definition: Tool,
    group: Group,
    access: Access,
    handler: Handler,
}

/// Listing and invocation share the same permission-filtered registry.
/// Each operation has its own access level, even when config groups are merged.
pub struct McpServer {
    config: Config,
    tools: BTreeMap<String, RegisteredTool>,
}

impl McpServer {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            tools: BTreeMap::new(),
        }
    }

    /// Register one reviewed operation. Handlers must return structured results
    /// and must never write to stdout. Configuration cannot add registrations.
    pub fn register<F, Fut>(
        &mut self,
        definition: Tool,
        group: Group,
        access: Access,
        handler: F,
    ) -> Result<()>
    where
        F: Fn(CallToolRequestParams, RequestContext<RoleServer>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<CallToolResponse, ErrorData>> + Send + 'static,
    {
        let name = definition.name.to_string();
        ensure!(
            !self.tools.contains_key(&name),
            "duplicate MCP tool '{name}'"
        );
        self.tools.insert(
            name,
            RegisteredTool {
                definition,
                group,
                access,
                handler: Box::new(move |request, context| Box::pin(handler(request, context))),
            },
        );
        Ok(())
    }

    fn allowed_tool(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools
            .get(name)
            .filter(|tool| self.config.allows(tool.group, tool.access))
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(crate::APP_NAME, env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            self.tools
                .keys()
                .filter_map(|name| self.get_tool(name))
                .collect(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.allowed_tool(name).map(|tool| tool.definition.clone())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = self.allowed_tool(&request.name).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                "unknown or disabled tool",
                None,
            )
        })?;
        (tool.handler)(request, context).await
    }
}

pub async fn run(args: McpArgs, global: &GlobalArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    for warning in config.warnings() {
        // These startup warnings are required even with --quiet or RUST_LOG=off.
        eprintln!("warning: {warning}");
    }

    let mut server = McpServer::new(config);
    tools::register(
        &mut server,
        std::sync::Arc::new(backend::DeviceBackend::new(global)?),
    )?;
    let service = server
        .serve(rmcp::transport::stdio_pipes())
        .await
        .wrap_err("failed to initialize MCP stdio server")?;
    service.waiting().await.wrap_err("MCP server task failed")?;
    Ok(())
}

#[cfg(test)]
mod pizzamat_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod history_tests;
