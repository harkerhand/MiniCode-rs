use anyhow::Result;
use minicode_config::{
    MiniCodeSettings, claude_settings_path, load_runtime_config, mini_code_mcp_path,
    mini_code_permissions_path, mini_code_settings_path, save_minicode_settings,
};
use minicode_history::{clear_history_entries, clear_runtime_messages_keep_system};
use minicode_tool::{TOOL_COMMANDS, ToolRegistry};

pub struct SlashCommand {
    pub prefix: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    pub handler: fn(&str, &std::path::Path, &ToolRegistry) -> Result<String>,
}

pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        prefix: "/help",
        usage: "/help",
        description: "显示可用斜杠命令。",
        handler: |_, _, _| Ok(format_slash_commands().join("\n")),
    },
    SlashCommand {
        prefix: "/tools",
        usage: "/tools",
        description: "列出可用工具。",
        handler: |_, _, tools| {
            Ok(tools
                .list()
                .iter()
                .map(|tool| format!("{}: {}", tool.name(), tool.description()))
                .collect::<Vec<_>>()
                .join("\n"))
        },
    },
    SlashCommand {
        prefix: "/status",
        usage: "/status",
        description: "显示当前模型与配置来源。",
        handler: |_, cwd, _| {
            let runtime = load_runtime_config(cwd)?;
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
                runtime.source_summary,
            ]
            .join("\n"))
        },
    },
    SlashCommand {
        prefix: "/model ",
        usage: "/model <model-name>",
        description: "保存模型覆盖到 ~/.mini-code/settings.json。",
        handler: |input, cwd, _| {
            let model = input.trim_start_matches("/model ");
            if model.is_empty() {
                return Err(anyhow::anyhow!("Model name is required."));
            }
            let mut runtime = load_runtime_config(cwd)?;
            runtime.model = model.to_string();
            save_minicode_settings(MiniCodeSettings {
                model: Some(model.to_string()),
                ..Default::default()
            })?;
            Ok(format!("Model updated to: {}", runtime.model))
        },
    },
    SlashCommand {
        prefix: "/model",
        usage: "/model",
        description: "显示当前模型。",
        handler: |_, cwd, _| {
            let runtime = load_runtime_config(cwd)?;
            Ok(format!("current model: {}", runtime.model))
        },
    },
    SlashCommand {
        prefix: "/config-paths",
        usage: "/config-paths",
        description: "显示配置文件路径。",
        handler: |cwd, _, _| {
            Ok([
                format!(
                    "mini-code settings: {}",
                    mini_code_settings_path().display()
                ),
                format!(
                    "mini-code permissions: {}",
                    mini_code_permissions_path(cwd).display()
                ),
                format!("mini-code mcp: {}", mini_code_mcp_path().display()),
                format!("compat fallback: {}", claude_settings_path().display()),
            ]
            .join("\n"))
        },
    },
    SlashCommand {
        prefix: "/skills",
        usage: "/skills",
        description: "列出已发现技能。",
        handler: |_, _, tools| {
            let skills = tools.get_skills();
            if skills.is_empty() {
                return Ok("No skills discovered.".to_string());
            }
            Ok(skills
                .iter()
                .map(|s| format!("{}  {}  [{}]", s.name, s.description, s.source))
                .collect::<Vec<_>>()
                .join("\n"))
        },
    },
    SlashCommand {
        prefix: "/mcp",
        usage: "/mcp",
        description: "显示 MCP 服务状态。",
        handler: |_, _, tools| {
            let servers = tools.get_mcp_servers();
            if servers.is_empty() {
                return Ok("No MCP servers configured.".to_string());
            }
            Ok(servers
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
                .join("\n"))
        },
    },
    SlashCommand {
        prefix: "/permissions",
        usage: "/permissions",
        description: "显示权限存储路径。",
        handler: |_, cwd, _| {
            Ok(format!(
                "permission store: {}",
                mini_code_permissions_path(cwd).display()
            ))
        },
    },
    SlashCommand {
        prefix: "/clear",
        usage: "/clear",
        description: "清空当前会话上下文（保留 system prompt）。",
        handler: |_, _, _| {
            clear_runtime_messages_keep_system();
            clear_history_entries()?;
            Ok("上下文已清空（保留 system prompt）。".to_string())
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
pub async fn try_handle_local_command(
    input: &str,
    cwd: &std::path::Path,
    tools: &ToolRegistry,
) -> Result<Option<String>> {
    for cmd in SLASH_COMMANDS {
        if input.starts_with(cmd.prefix) {
            let result = (cmd.handler)(input, cwd, tools)?;
            return Ok(Some(result));
        }
    }
    Ok(None)
}
