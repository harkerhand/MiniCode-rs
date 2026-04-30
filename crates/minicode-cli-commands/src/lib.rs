use anyhow::Result;
use minicode_config::{
    mini_code_mcp_path, mini_code_permissions_path, mini_code_settings_path, runtime_config,
    save_minicode_settings,
};
use minicode_history::{clear_history_entries, clear_runtime_messages};
use minicode_tool::{TOOL_COMMANDS, get_tool_registry};
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct SlashCommand {
    pub prefix: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    pub handler: fn(&str) -> BoxFuture<'static, Result<String>>,
}

pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        prefix: "/help",
        usage: "/help",
        description: "显示可用斜杠命令。",
        handler: |_| Box::pin(async move { Ok(format_slash_commands().join("\n")) }),
    },
    SlashCommand {
        prefix: "/tools",
        usage: "/tools",
        description: "列出可用工具。",
        handler: |_| {
            let tools = get_tool_registry();
            let str = tools
                .list()
                .iter()
                .map(|tool| format!("{}: {}", tool.name(), tool.description()))
                .collect::<Vec<_>>()
                .join("\n");
            Box::pin(async move { Ok(str) })
        },
    },
    SlashCommand {
        prefix: "/status",
        usage: "/status",
        description: "显示当前模型与配置来源。",
        handler: |_| {
            Box::pin(async move {
                let runtime = runtime_config();
                let auth = if runtime.auth_token.is_some() {
                    "ANTHROPIC_AUTH_TOKEN"
                } else {
                    "ANTHROPIC_API_KEY"
                };
                Ok([
                    format!("model: {}", runtime.model),
                    format!("baseUrl: {}", runtime.base_url),
                    format!("auth: {auth}"),
                    format!("mcp servers: {}", runtime.mcp_servers.len()),
                ]
                .join("\n"))
            })
        },
    },
    SlashCommand {
        prefix: "/model ",
        usage: "/model <model-name>",
        description: "保存模型覆盖到 ~/.mini-code/settings.json。",
        handler: |input| {
            let model = input.trim_start_matches("/model ").to_string();
            Box::pin(async move {
                if model.is_empty() {
                    return Err(anyhow::anyhow!("Model name is required."));
                }
                let mut runtime = runtime_config();
                runtime.model = model.to_string();
                save_minicode_settings(&runtime)?;
                Ok(format!("Model updated to: {}", runtime.model))
            })
        },
    },
    SlashCommand {
        prefix: "/model",
        usage: "/model",
        description: "显示当前模型。",
        handler: |_| {
            Box::pin(async move {
                let runtime = runtime_config();
                Ok(format!("current model: {}", runtime.model))
            })
        },
    },
    SlashCommand {
        prefix: "/config-paths",
        usage: "/config-paths",
        description: "显示配置文件路径。",
        handler: |_| {
            Box::pin(async move {
                Ok([
                    format!(
                        "mini-code settings: {}",
                        mini_code_settings_path().display()
                    ),
                    format!(
                        "mini-code permissions: {}",
                        mini_code_permissions_path().display()
                    ),
                    format!("mini-code mcp: {}", mini_code_mcp_path().display()),
                ]
                .join("\n"))
            })
        },
    },
    SlashCommand {
        prefix: "/skills",
        usage: "/skills",
        description: "列出已发现技能。",
        handler: |_| {
            let tools = get_tool_registry();
            let skills = tools.get_skills();
            let str = skills
                .iter()
                .map(|s| format!("{}  {}  [{}]", s.name, s.description, s.source))
                .collect::<Vec<_>>()
                .join("\n");

            Box::pin(async move { Ok(str) })
        },
    },
    SlashCommand {
        prefix: "/mcp",
        usage: "/mcp",
        description: "显示 MCP 服务状态。",
        handler: |_| {
            let tools = get_tool_registry();
            let servers = tools.get_mcp_servers();
            let str = servers
                .iter()
                .map(|s| {
                    let protocol = s
                        .protocol
                        .as_ref()
                        .map(|x| format!("  protocol={x}"))
                        .unwrap_or_default();
                    let resources = s
                        .resource_count
                        .map(|x| format!("  resources={x}"))
                        .unwrap_or_default();
                    let prompts = s
                        .prompt_count
                        .map(|x| format!("  prompts={x}"))
                        .unwrap_or_default();
                    format!(
                        "{}  status={}  tools={}{}{}{}{}",
                        s.name,
                        s.status,
                        s.tool_count,
                        resources,
                        prompts,
                        protocol,
                        s.error
                            .as_ref()
                            .map(|x| format!("  error={x}"))
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Box::pin(async move { Ok(str) })
        },
    },
    SlashCommand {
        prefix: "/permissions",
        usage: "/permissions",
        description: "显示权限存储路径。",
        handler: |_| {
            Box::pin(async move {
                Ok(format!(
                    "permission store: {}",
                    mini_code_permissions_path().display()
                ))
            })
        },
    },
    SlashCommand {
        prefix: "/clear",
        usage: "/clear",
        description: "清空当前会话上下文（保留 system prompt）。",
        handler: |_| {
            Box::pin(async move {
                clear_runtime_messages();
                clear_history_entries()?;
                Ok("上下文已清空".to_string())
            })
        },
    },
    SlashCommand {
        prefix: "/compact",
        usage: "/compact",
        description: "手动压缩当前会话上下文。",
        handler: |_| {
            Box::pin(async move {
                let messages_without_system =
                    minicode_history::runtime_messages_for_context();
                let mut messages = Vec::with_capacity(messages_without_system.len() + 1);
                messages.push(minicode_types::ChatMessage::System {
                    content: minicode_prompt::build_system_prompt(),
                });
                messages.extend(messages_without_system);

                let count_before = messages.len();
                let model = minicode_types::get_model_adapter();
                let compacted = minicode_agent_core::maybe_auto_compact_conversation(
                    model.as_ref(),
                    messages,
                    Some(0), // 强制压缩，不检查阈值
                    Some(2),  // 保留最近 2 条
                    None::<&(dyn Fn(&str) + Send + Sync)>,
                )
                .await;

                if compacted.len() < count_before {
                    let arc = minicode_history::get_messages();
                    let mut guard = match arc.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let system_msgs: Vec<minicode_types::ChatMessage> = guard
                        .iter()
                        .filter(|m| matches!(m, minicode_types::ChatMessage::System { .. }))
                        .cloned()
                        .collect();
                    guard.clear();
                    guard.extend(system_msgs);
                    for msg in &compacted {
                        if !matches!(msg, minicode_types::ChatMessage::System { .. }) {
                            guard.push(msg.clone());
                        }
                    }
                    // 持久化压缩后的消息
                    drop(guard);
                    minicode_history::persist_current_messages();
                    Ok(format!(
                        "上下文已压缩：{} 条消息 -> {} 条",
                        count_before,
                        compacted.len()
                    ))
                } else {
                    Ok("当前上下文较短，无需压缩。".to_string())
                }
            })
        },
    },
];

/// 格式化所有内置斜杠命令的帮助文本。
pub fn format_slash_commands() -> Vec<String> {
    let slash_commands_info = SLASH_COMMANDS
        .iter()
        .map(|x| format!("{}  {}", x.usage, x.description));
    let tool_commands_info = TOOL_COMMANDS
        .iter()
        .map(|x| format!("{}  {}", x.usage, x.description));
    slash_commands_info
        .chain(tool_commands_info)
        .collect::<Vec<_>>()
}

/// 根据输入前缀返回可匹配的斜杠命令。
pub fn find_matching_slash_commands(input: &str) -> Vec<(String, String)> {
    let slash_commands = SLASH_COMMANDS
        .iter()
        .filter(|cmd| cmd.usage.starts_with(input))
        .map(|cmd| (cmd.usage.to_string(), cmd.description.to_string()));
    let tool_commands = TOOL_COMMANDS
        .iter()
        .filter(|cmd| cmd.usage.starts_with(input))
        .map(|cmd| (cmd.usage.to_string(), cmd.description.to_string()));
    slash_commands.chain(tool_commands).collect()
}

/// 尝试处理本地斜杠命令，无法处理时返回 `None`。
pub async fn try_handle_local_command(input: &str) -> Result<Option<String>> {
    for cmd in SLASH_COMMANDS {
        if input.starts_with(cmd.prefix) {
            let result = (cmd.handler)(input).await?;
            return Ok(Some(result));
        }
    }
    Ok(None)
}
