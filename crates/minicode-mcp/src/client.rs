use std::collections::HashMap;

use minicode_config::McpServerConfig;
use minicode_tool::ToolResult;
use rmcp::model::{CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams};
use rmcp::service::RunningService;
use serde_json::Value;

use crate::content_length_transport::start_content_length_service;
use crate::logging::mcp_log;
use crate::mcp_tools::{
    self, McpPromptDescriptor, McpResourceDescriptor, McpToolDescriptor, format_tool_result,
};
use crate::newline_json_transport::start_newline_json_service;
use crate::streamable_http_transport::start_streamable_http_service;

pub(crate) struct McpClient {
    pub(crate) server_name: String,
    protocol: &'static str,
    service: RunningService<rmcp::RoleClient, ()>,
}

impl McpClient {
    pub(crate) async fn start(
        server_name: &str,
        config: &McpServerConfig,
        cwd: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let command = config.command.trim();
        let url = config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty());
        let protocol_hint = config.protocol.as_deref();
        let selected_protocol = match protocol_hint {
            Some("auto") | None => {
                if url.is_some() {
                    "streamable-http"
                } else {
                    "newline-json"
                }
            }
            Some(value) => value,
        };

        let (service, protocol) = match selected_protocol {
            "streamable-http" => {
                let endpoint = url.ok_or_else(|| {
                    anyhow::anyhow!(
                        "MCP server {} uses streamable-http protocol but `url` is empty",
                        server_name
                    )
                })?;
                let headers = extract_string_map(config.headers.as_ref())?;
                mcp_log(format!(
                    "server={} rmcp connect remote url={} headers={}",
                    server_name,
                    endpoint,
                    headers.len()
                ));
                let service =
                    start_streamable_http_service(endpoint, &headers, server_name).await?;
                (service, "streamable-http(rmcp)")
            }
            "content-length" | "newline-json" => {
                if command.is_empty() {
                    return Err(anyhow::anyhow!(
                        "MCP server {} has empty command for protocol {}",
                        server_name,
                        selected_protocol
                    ));
                }

                let mut cmd = tokio::process::Command::new(&config.command);
                cmd.args(config.args.clone().unwrap_or_default())
                    .current_dir(if let Some(custom) = &config.cwd {
                        cwd.join(custom)
                    } else {
                        cwd.to_path_buf()
                    });

                if let Some(envs) = &config.env {
                    for (k, v) in envs {
                        cmd.env(k, v.to_string().trim_matches('"'));
                    }
                }

                mcp_log(format!(
                    "server={} rmcp spawn command={} args={:?}",
                    server_name,
                    config.command,
                    config.args.clone().unwrap_or_default()
                ));

                if selected_protocol == "content-length" {
                    (
                        start_content_length_service(cmd, server_name, &config.command).await?,
                        "content-length(rmcp-compat)",
                    )
                } else {
                    (
                        start_newline_json_service(cmd, server_name, &config.command).await?,
                        "newline-json(rmcp)",
                    )
                }
            }
            other => {
                return Err(anyhow::anyhow!(
                    "MCP server {} uses unsupported protocol `{}`; expected `auto`, `newline-json`, `content-length`, or `streamable-http`",
                    server_name,
                    other
                ));
            }
        };

        Ok(Self {
            server_name: server_name.to_string(),
            protocol,
            service,
        })
    }

    pub(crate) fn protocol_name(&self) -> &'static str {
        self.protocol
    }

    pub(crate) async fn list_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>> {
        let tools = self.service.list_all_tools().await?;
        Ok(tools
            .into_iter()
            .map(|tool| McpToolDescriptor {
                name: tool.name.into_owned(),
                description: tool.description.map(|v| v.into_owned()),
                input_schema: Some(Value::Object((*tool.input_schema).clone())),
            })
            .collect())
    }

    pub(crate) async fn list_resources(&self) -> anyhow::Result<Vec<McpResourceDescriptor>> {
        let resources = self.service.list_all_resources().await?;
        Ok(resources
            .into_iter()
            .map(|resource| McpResourceDescriptor {
                uri: resource.uri.clone(),
                name: Some(resource.name.clone()),
                description: resource.description.clone(),
            })
            .collect())
    }

    pub(crate) async fn list_prompts(&self) -> anyhow::Result<Vec<McpPromptDescriptor>> {
        let prompts = self.service.list_all_prompts().await?;
        Ok(prompts
            .into_iter()
            .map(|prompt| McpPromptDescriptor {
                name: prompt.name,
                description: prompt.description,
                arguments: prompt.arguments.map(|args| {
                    args.into_iter()
                        .map(|arg| mcp_tools::McpPromptArg {
                            name: arg.name,
                            required: arg.required,
                        })
                        .collect()
                }),
            })
            .collect())
    }

    pub(crate) async fn call_tool(&self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
        let mut params = CallToolRequestParams::new(name.to_string());
        if !input.is_null() {
            let args = serde_json::from_value(input)
                .map_err(|_| anyhow::anyhow!("tool input must be a JSON object"))?;
            params = params.with_arguments(args);
        }
        let result = self.service.call_tool(params).await?;
        let raw = serde_json::to_value(result)?;
        Ok(format_tool_result(raw))
    }

    pub(crate) async fn read_resource(&self, uri: &str) -> anyhow::Result<ToolResult> {
        let result = self
            .service
            .read_resource(ReadResourceRequestParams::new(uri))
            .await?;
        Ok(ToolResult::ok(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{result:?}")),
        ))
    }

    pub(crate) async fn get_prompt(&self, name: &str, args: Value) -> anyhow::Result<ToolResult> {
        let mut params = GetPromptRequestParams::new(name.to_string());
        if !args.is_null() {
            let args = serde_json::from_value(args)
                .map_err(|_| anyhow::anyhow!("prompt arguments must be a JSON object"))?;
            params = params.with_arguments(args);
        }
        let result = self.service.get_prompt(params).await?;
        Ok(ToolResult::ok(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{result:?}")),
        ))
    }

    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        mcp_log(format!("server={} rmcp closing process", self.server_name));
        let _ = self.service.close().await;
        mcp_log(format!("server={} rmcp closed", self.server_name));
        Ok(())
    }
}

fn extract_string_map(
    values: Option<&HashMap<String, serde_json::Value>>,
) -> anyhow::Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    let Some(values) = values else {
        return Ok(result);
    };

    for (key, value) in values {
        let parsed = value
            .as_str()
            .map(|x| x.to_string())
            .unwrap_or_else(|| value.to_string().trim_matches('"').to_string());
        result.insert(key.clone(), parsed);
    }
    Ok(result)
}
