use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use minicode_config::McpServerConfig;
use minicode_tool::Tool;
use minicode_types::McpServerSummary;
use tokio::sync::Mutex;

use crate::client::McpClient;
use crate::logging::mcp_log;
use crate::mcp_tools::{
    McpPromptDescriptor, McpResourceDescriptor, McpToolDescriptor, append_dynamic_tools,
    append_resource_prompt_tools,
};
use crate::{MCP_LIST_TIMEOUT, MCP_STARTUP_TIMEOUT, McpBundle};

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
