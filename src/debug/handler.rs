//! `ServerHandler` implementation for `DebugServer`.

use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    ErrorData as McpError, RoleServer, ServerHandler,
};
use std::sync::atomic::Ordering;

use super::proxy;
use super::server::DebugServer;

impl ServerHandler for DebugServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "programmer-mcp-debug",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Debug control server for programmer-mcp.\n\
                 Commands: `rebuild`, `update_debug_bin`, `status`, `configure`, \
                 `show_config`, `grab_log`, `show_help`, `execute`.\n\
                 Use `show_help` to inspect the child's tool list, then use `execute` \
                 with DSL commands to interact with the running child.",
            )
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if self.proxy_mode.load(Ordering::Relaxed) && request.name != "update_debug_bin" {
            return proxy::proxy_call_tool(
                &self.debug_child,
                &self.next_id,
                &self.proxy_mode,
                request,
            )
            .await;
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        if self.proxy_mode.load(Ordering::Relaxed) {
            return proxy::proxy_list_tools(&self.debug_child, &self.next_id, &self.proxy_mode)
                .await;
        }
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tool_router.get(name).cloned()
    }
}
