//! Carries MCP over Streamable HTTP to the tools. It answers every POST on
//! its own (no sessions) with a JSON body, and decides nothing about tools
//! or the vault.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use tools::Tools;

/// The MCP face of the tools.
#[derive(Clone)]
pub struct Server {
    tools: Arc<Tools>,
}

impl Server {
    pub fn new(tools: Tools) -> Server {
        Server { tools: Arc::new(tools) }
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("obsidian-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Tools for one Obsidian vault, in Obsidian's terms: notes, links, sections, properties, tags. \
                 Each result is one JSON object; a long result carries next_page to pass back as page.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = tools::specs()
            .into_iter()
            .map(|spec| {
                let schema = match spec.input_schema {
                    serde_json::Value::Object(map) => map,
                    _ => Default::default(),
                };
                let mut tool = Tool::new(spec.name, spec.description, schema);
                tool.annotations = Some(ToolAnnotations::new().read_only(spec.read_only));
                tool
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tools = self.tools.clone();
        let name = request.name.to_string();
        let args = request.arguments.map(serde_json::Value::Object);
        // Tools read the vault from disk, so they run off the async workers.
        let reply = tokio::task::spawn_blocking(move || tools.call(&name, args))
            .await
            .map_err(|e| ErrorData::internal_error(format!("tool task failed: {e}"), None))?;
        let content = vec![ContentBlock::text(reply.text)];
        let result = if reply.is_error { CallToolResult::error(content) } else { CallToolResult::success(content) };
        Ok(result.into())
    }
}

/// The `/mcp` service: stateless, JSON responses. `allowed_hosts` are the
/// `Host` values accepted besides loopback (for a public hostname behind a
/// tunnel).
pub fn mcp_service(tools: Tools, allowed_hosts: &[String]) -> StreamableHttpService<Server, NeverSessionManager> {
    let server = Server::new(tools);
    let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
    hosts.extend(allowed_hosts.iter().cloned());
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts(hosts);
    StreamableHttpService::new(move || Ok(server.clone()), Arc::new(NeverSessionManager::default()), config)
}

pub fn router(tools: Tools, allowed_hosts: &[String]) -> axum::Router {
    axum::Router::new().nest_service("/mcp", mcp_service(tools, allowed_hosts))
}
