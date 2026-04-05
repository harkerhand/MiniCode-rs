use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use mcp_tools::{
    McpPromptDescriptor, McpResourceDescriptor, McpToolDescriptor, append_dynamic_tools,
    append_resource_prompt_tools, format_tool_result,
};
use minicode_config::McpServerConfig;
use minicode_tool::{Tool, ToolRegistry, ToolResult};
use minicode_types::{McpServerSummary, SkillSummary};
use rmcp::model::{CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams};
use rmcp::service::RunningService;
use serde_json::Value;
use tokio::sync::Mutex;

mod content_length_transport;
mod mcp_tools;
mod newline_json_transport;
mod streamable_http_transport;

use content_length_transport::start_content_length_service;
use newline_json_transport::start_newline_json_service;
use streamable_http_transport::start_streamable_http_service;

const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(3);
static MCP_LOG_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_mcp_logging_enabled(enabled: bool) {
    MCP_LOG_ENABLED.store(enabled, Ordering::Relaxed);
}

fn mcp_log(message: impl AsRef<str>) {
    if !MCP_LOG_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    eprintln!("\x1b[32m[mcp]\x1b[0m {}", message.as_ref());
}

pub struct McpBundle {
    pub tools: Vec<Arc<dyn Tool>>,
    pub servers: Vec<McpServerSummary>,
    pub disposer: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,
}

struct McpClient {
    server_name: String,
    protocol: &'static str,
    service: RunningService<rmcp::RoleClient, ()>,
}

impl McpClient {
    async fn start(
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

    fn protocol_name(&self) -> &'static str {
        self.protocol
    }

    async fn list_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>> {
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

    async fn list_resources(&self) -> anyhow::Result<Vec<McpResourceDescriptor>> {
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

    async fn list_prompts(&self) -> anyhow::Result<Vec<McpPromptDescriptor>> {
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

    async fn call_tool(&self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
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

    async fn read_resource(&self, uri: &str) -> anyhow::Result<ToolResult> {
        let result = self
            .service
            .read_resource(ReadResourceRequestParams::new(uri))
            .await?;
        Ok(ToolResult::ok(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{result:?}")),
        ))
    }

    async fn get_prompt(&self, name: &str, args: Value) -> anyhow::Result<ToolResult> {
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

    async fn close(&mut self) -> anyhow::Result<()> {
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

fn summarize_server_endpoint(config: &McpServerConfig) -> String {
    if let Some(url) = config
        .url
        .as_deref()
        .map(str::trim)
        .filter(|x| !x.is_empty())
    {
        url.to_string()
    } else {
        config.command.clone()
    }
}

pub async fn create_mcp_backed_tools(
    cwd: &std::path::Path,
    mcp_servers: &HashMap<String, McpServerConfig>,
) -> McpBundle {
    let mut tools: Vec<Arc<dyn Tool>> = vec![];
    let mut servers = vec![];
    let mut clients: HashMap<String, Arc<Mutex<McpClient>>> = HashMap::new();
    let mut resource_entries: Vec<(String, McpResourceDescriptor)> = vec![];
    let mut prompt_entries: Vec<(String, McpPromptDescriptor)> = vec![];
    let mut closers: Vec<Arc<Mutex<McpClient>>> = vec![];

    struct ConnectSuccess {
        server_name: String,
        config: McpServerConfig,
        client: McpClient,
        tool_descriptors: Vec<McpToolDescriptor>,
        resources: Vec<McpResourceDescriptor>,
        prompts: Vec<McpPromptDescriptor>,
        protocol: String,
    }

    enum ConnectOutcome {
        Success(ConnectSuccess),
        Failure {
            server_name: String,
            config: McpServerConfig,
            error: String,
        },
    }

    let mut pending = FuturesUnordered::new();

    for (server_name, config) in mcp_servers {
        mcp_log(format!("bootstrap begin server={}", server_name));
        if config.enabled == Some(false) {
            servers.push(McpServerSummary {
                name: server_name.clone(),
                command: summarize_server_endpoint(config),
                status: "disabled".to_string(),
                tool_count: 0,
                error: None,
                protocol: config.protocol.clone(),
                resource_count: Some(0),
                prompt_count: Some(0),
            });
            continue;
        }

        let server_name = server_name.clone();
        let config = config.clone();
        let cwd = cwd.to_path_buf();
        pending.push(async move {
            let failure_server_name = server_name.clone();
            let failure_config = config.clone();
            let task = async {
                let client = McpClient::start(&server_name, &config, &cwd).await?;
                let protocol = client.protocol_name().to_string();
                mcp_log(format!(
                    "bootstrap server={} connected protocol={}, listing tools/resources/prompts",
                    server_name, protocol
                ));

                let tool_descriptors = client.list_tools().await.unwrap_or_default();
                let resources =
                    match tokio::time::timeout(MCP_LIST_TIMEOUT, client.list_resources()).await {
                        Ok(Ok(v)) => v,
                        _ => vec![],
                    };
                let prompts =
                    match tokio::time::timeout(MCP_LIST_TIMEOUT, client.list_prompts()).await {
                        Ok(Ok(v)) => v,
                        _ => vec![],
                    };

                Ok::<ConnectSuccess, anyhow::Error>(ConnectSuccess {
                    server_name,
                    config,
                    client,
                    tool_descriptors,
                    resources,
                    prompts,
                    protocol,
                })
            };

            match tokio::time::timeout(MCP_STARTUP_TIMEOUT, task).await {
                Ok(Ok(success)) => ConnectOutcome::Success(success),
                Ok(Err(err)) => ConnectOutcome::Failure {
                    server_name: failure_server_name.clone(),
                    config: failure_config.clone(),
                    error: err.to_string(),
                },
                Err(_) => ConnectOutcome::Failure {
                    server_name: failure_server_name,
                    config: failure_config,
                    error: format!(
                        "MCP startup timed out after {}s (try pre-installing the server package or increasing network speed)",
                        MCP_STARTUP_TIMEOUT.as_secs(),
                    ),
                },
            }
        });
    }

    while let Some(outcome) = pending.next().await {
        match outcome {
            ConnectOutcome::Success(success) => {
                mcp_log(format!(
                    "bootstrap success server={} tools={} resources={} prompts={}",
                    success.server_name,
                    success.tool_descriptors.len(),
                    success.resources.len(),
                    success.prompts.len()
                ));
                let server_name = success.server_name;
                let config = success.config;
                let tool_descriptors = success.tool_descriptors;
                let resources = success.resources;
                let prompts = success.prompts;
                let client = Arc::new(Mutex::new(success.client));

                clients.insert(server_name.clone(), client.clone());
                closers.push(client.clone());

                append_dynamic_tools(&mut tools, &server_name, &tool_descriptors, &client);

                for resource in resources.clone() {
                    resource_entries.push((server_name.clone(), resource));
                }
                for prompt in prompts.clone() {
                    prompt_entries.push((server_name.clone(), prompt));
                }

                servers.push(McpServerSummary {
                    name: server_name,
                    command: summarize_server_endpoint(&config),
                    status: "connected".to_string(),
                    tool_count: tool_descriptors.len(),
                    error: None,
                    protocol: Some(success.protocol),
                    resource_count: Some(resources.len()),
                    prompt_count: Some(prompts.len()),
                });
            }
            ConnectOutcome::Failure {
                server_name,
                config,
                error,
            } => {
                mcp_log(format!(
                    "bootstrap failure server={} error={}",
                    server_name, error
                ));
                servers.push(McpServerSummary {
                    name: server_name,
                    command: summarize_server_endpoint(&config),
                    status: "error".to_string(),
                    tool_count: 0,
                    error: Some(error),
                    protocol: config.protocol,
                    resource_count: Some(0),
                    prompt_count: Some(0),
                });
            }
        }
    }

    append_resource_prompt_tools(&mut tools, resource_entries, prompt_entries, &clients);

    let disposer = if closers.is_empty() {
        None
    } else {
        let closers = Arc::new(closers);
        Some(Arc::new(move || {
            let closers = closers.clone();
            let fut: BoxFuture<'static, ()> = Box::pin(async move {
                for client in closers.iter() {
                    let mut inner = client.lock().await;
                    let _ = inner.close().await;
                }
            });
            fut
        })
            as Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>)
    };

    McpBundle {
        tools,
        servers,
        disposer,
    }
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
