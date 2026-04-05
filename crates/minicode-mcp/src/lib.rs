use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use minicode_tool::{Tool, ToolRegistry};
use minicode_types::{McpServerSummary, SkillSummary};

mod bootstrap;
mod client;
mod content_length_transport;
mod logging;
mod mcp_tools;
mod newline_json_transport;
mod streamable_http_transport;

pub use bootstrap::create_mcp_backed_tools;
pub use logging::set_mcp_logging_enabled;

pub(crate) const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
pub(crate) const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(3);

pub struct McpBundle {
    pub tools: Vec<Arc<dyn Tool>>,
    pub servers: Vec<McpServerSummary>,
    pub disposer: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,
}

pub fn extend_registry_with_mcp(
    tools: Vec<Arc<dyn Tool>>,
    skills: Vec<SkillSummary>,
    mcp: McpBundle,
) -> ToolRegistry {
    let mut merged = tools;
    merged.extend(mcp.tools);
    ToolRegistry::new(merged, skills, mcp.servers, mcp.disposer)
}
