use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use minicode_tool::{Tool, ToolContext, ToolResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::McpClient;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct McpToolDescriptor {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub(crate) input_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct McpResourceDescriptor {
    pub(crate) uri: String,
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct McpPromptArg {
    pub(crate) name: String,
    pub(crate) required: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct McpPromptDescriptor {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) arguments: Option<Vec<McpPromptArg>>,
}

fn sanitize_segment(value: &str) -> String {
    let mut s = value
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    s = s.trim_matches('_').to_string();
    if s.is_empty() { "tool".to_string() } else { s }
}

pub(crate) fn format_tool_result(result: Value) -> ToolResult {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut parts = vec![];
    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        for block in content {
            if block.get("type").and_then(|v| v.as_str()) == Some("text")
                && let Some(text) = block.get("text").and_then(|v| v.as_str())
            {
                parts.push(text.to_string());
                continue;
            }

            parts.push(serde_json::to_string_pretty(block).unwrap_or_else(|_| block.to_string()));
        }
    }
    if let Some(structured) = result.get("structuredContent") {
        parts.push(format!(
            "STRUCTURED_CONTENT:\n{}",
            serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string())
        ));
    }
    if parts.is_empty() {
        parts.push(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()));
    }

    ToolResult {
        ok: !is_error,
        output: parts.join("\n\n"),
        background_task: None,
        await_user: false,
    }
}

pub(crate) fn append_dynamic_tools(
    tools: &mut Vec<Arc<dyn Tool>>,
    server_name: &str,
    descriptors: &[McpToolDescriptor],
    client: &Arc<Mutex<McpClient>>,
) {
    for descriptor in descriptors {
        let wrapped_name = format!(
            "mcp__{}__{}",
            sanitize_segment(server_name),
            sanitize_segment(&descriptor.name)
        );
        tools.push(Arc::new(McpDynamicTool {
            wrapped_name,
            description: descriptor.description.clone().unwrap_or_else(|| {
                format!(
                    "Call MCP tool {} from server {}.",
                    descriptor.name, server_name
                )
            }),
            input_schema: descriptor
                .input_schema
                .clone()
                .unwrap_or_else(|| json!({"type":"object","additionalProperties":true})),
            tool_name: descriptor.name.clone(),
            client: client.clone(),
        }));
    }
}

pub(crate) fn append_resource_prompt_tools(
    tools: &mut Vec<Arc<dyn Tool>>,
    resource_entries: Vec<(String, McpResourceDescriptor)>,
    prompt_entries: Vec<(String, McpPromptDescriptor)>,
    clients: &HashMap<String, Arc<Mutex<McpClient>>>,
) {
    if !resource_entries.is_empty() {
        tools.push(Arc::new(ListMcpResourcesTool {
            entries: resource_entries,
        }));
        tools.push(Arc::new(ReadMcpResourceTool {
            clients: clients.clone(),
        }));
    }

    if !prompt_entries.is_empty() {
        tools.push(Arc::new(ListMcpPromptsTool {
            entries: prompt_entries,
        }));
        tools.push(Arc::new(GetMcpPromptTool {
            clients: clients.clone(),
        }));
    }
}

struct McpDynamicTool {
    wrapped_name: String,
    description: String,
    input_schema: Value,
    tool_name: String,
    client: Arc<Mutex<McpClient>>,
}

#[async_trait]
impl Tool for McpDynamicTool {
    fn name(&self) -> &str {
        &self.wrapped_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let client = self.client.lock().await;
        match client.call_tool(&self.tool_name, input).await {
            Ok(result) => result,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

struct ListMcpResourcesTool {
    entries: Vec<(String, McpResourceDescriptor)>,
}

#[async_trait]
impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "list_mcp_resources"
    }

    fn description(&self) -> &str {
        "列出当前已连接 MCP 服务提供的资源。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"}}})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server_filter = input.get("server").and_then(|v| v.as_str());
        let lines = self
            .entries
            .iter()
            .filter(|(server, _)| match server_filter {
                Some(f) => f == server,
                None => true,
            })
            .map(|(server, resource)| {
                format!(
                    "{}: {}{}{}",
                    server,
                    resource.uri,
                    resource
                        .name
                        .as_ref()
                        .map(|x| format!(" ({})", x))
                        .unwrap_or_default(),
                    resource
                        .description
                        .as_ref()
                        .map(|x| format!(" - {}", x))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();

        if lines.is_empty() {
            ToolResult::ok("No MCP resources available.")
        } else {
            ToolResult::ok(lines.join("\n"))
        }
    }
}

struct ReadMcpResourceTool {
    clients: HashMap<String, Arc<Mutex<McpClient>>>,
}

#[async_trait]
impl Tool for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "read_mcp_resource"
    }

    fn description(&self) -> &str {
        "读取指定 MCP 资源。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"},"uri":{"type":"string"}},"required":["server","uri"]})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server = input
            .get("server")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let uri = input
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if server.is_empty() || uri.is_empty() {
            return ToolResult::err("server/uri is required");
        }
        let Some(client) = self.clients.get(server) else {
            return ToolResult::err(format!("Unknown MCP server: {}", server));
        };
        let inner = client.lock().await;
        match inner.read_resource(uri).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

struct ListMcpPromptsTool {
    entries: Vec<(String, McpPromptDescriptor)>,
}

#[async_trait]
impl Tool for ListMcpPromptsTool {
    fn name(&self) -> &str {
        "list_mcp_prompts"
    }

    fn description(&self) -> &str {
        "列出当前已连接 MCP 服务提供的提示模板。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"}}})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server_filter = input.get("server").and_then(|v| v.as_str());
        let lines = self
            .entries
            .iter()
            .filter(|(server, _)| match server_filter {
                Some(f) => f == server,
                None => true,
            })
            .map(|(server, prompt)| {
                let args = prompt
                    .arguments
                    .as_ref()
                    .map(|x| {
                        x.iter()
                            .map(|a| {
                                format!(
                                    "{}{}",
                                    a.name,
                                    if a.required == Some(true) { "*" } else { "" }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!(
                    "{}: {}{}{}",
                    server,
                    prompt.name,
                    if args.is_empty() {
                        "".to_string()
                    } else {
                        format!(" args=[{}]", args)
                    },
                    prompt
                        .description
                        .as_ref()
                        .map(|x| format!(" - {}", x))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            ToolResult::ok("No MCP prompts available.")
        } else {
            ToolResult::ok(lines.join("\n"))
        }
    }
}

struct GetMcpPromptTool {
    clients: HashMap<String, Arc<Mutex<McpClient>>>,
}

#[async_trait]
impl Tool for GetMcpPromptTool {
    fn name(&self) -> &str {
        "get_mcp_prompt"
    }

    fn description(&self) -> &str {
        "渲染并获取 MCP Prompt。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"},"name":{"type":"string"},"arguments":{"type":"object","additionalProperties":{"type":"string"}}},"required":["server","name"]})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server = input
            .get("server")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if server.is_empty() || name.is_empty() {
            return ToolResult::err("server/name is required");
        }

        let args = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let Some(client) = self.clients.get(server) else {
            return ToolResult::err(format!("Unknown MCP server: {}", server));
        };
        let inner = client.lock().await;
        match inner.get_prompt(name, args).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
