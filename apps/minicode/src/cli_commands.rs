use anyhow::Result;

use crate::config::{
    MiniCodeSettings, claude_settings_path, load_runtime_config, mini_code_mcp_path,
    mini_code_permissions_path, mini_code_settings_path, save_minicode_settings,
};
use crate::tool::ToolRegistry;

pub struct SlashCommand {
    pub usage: &'static str,
    pub description: &'static str,
}

pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        usage: "/help",
        description: "显示可用斜杠命令。",
    },
    SlashCommand {
        usage: "/tools",
        description: "列出可用工具。",
    },
    SlashCommand {
        usage: "/status",
        description: "显示当前模型与配置来源。",
    },
    SlashCommand {
        usage: "/model",
        description: "显示当前模型。",
    },
    SlashCommand {
        usage: "/model <model-name>",
        description: "保存模型覆盖到 ~/.mini-code/settings.json。",
    },
    SlashCommand {
        usage: "/config-paths",
        description: "显示配置文件路径。",
    },
    SlashCommand {
        usage: "/skills",
        description: "列出已发现技能。",
    },
    SlashCommand {
        usage: "/mcp",
        description: "显示 MCP 服务状态。",
    },
    SlashCommand {
        usage: "/permissions",
        description: "显示权限存储路径。",
    },
    SlashCommand {
        usage: "/exit",
        description: "退出。",
    },
    SlashCommand {
        usage: "/ls [path]",
        description: "列出目录文件。",
    },
    SlashCommand {
        usage: "/grep <pattern>::[path]",
        description: "在文件中搜索文本。",
    },
    SlashCommand {
        usage: "/read <path>",
        description: "直接读取文件内容。",
    },
    SlashCommand {
        usage: "/write <path>::<content>",
        description: "直接写入文件。",
    },
    SlashCommand {
        usage: "/modify <path>::<content>",
        description: "替换文件内容（可审阅 diff）。",
    },
    SlashCommand {
        usage: "/edit <path>::<search>::<replace>",
        description: "按精确文本替换编辑文件。",
    },
    SlashCommand {
        usage: "/patch <path>::<search1>::<replace1>::...",
        description: "对单文件执行多组替换。",
    },
    SlashCommand {
        usage: "/cmd [cwd::]<command> [args...]",
        description: "直接执行允许列表内命令。",
    },
];

pub fn format_slash_commands() -> String {
    SLASH_COMMANDS
        .iter()
        .map(|x| format!("{}  {}", x.usage, x.description))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn find_matching_slash_commands(input: &str) -> Vec<String> {
    SLASH_COMMANDS
        .iter()
        .map(|x| x.usage.to_string())
        .filter(|x| x.starts_with(input))
        .collect()
}

pub async fn try_handle_local_command(
    input: &str,
    cwd: &std::path::Path,
    tools: Option<&ToolRegistry>,
) -> Result<Option<String>> {
    if input == "/" || input == "/help" {
        return Ok(Some(format_slash_commands()));
    }

    if input == "/config-paths" {
        return Ok(Some(
            vec![
                format!(
                    "mini-code settings: {}",
                    mini_code_settings_path().display()
                ),
                format!(
                    "mini-code permissions: {}",
                    mini_code_permissions_path().display()
                ),
                format!("mini-code mcp: {}", mini_code_mcp_path().display()),
                format!("compat fallback: {}", claude_settings_path().display()),
            ]
            .join("\n"),
        ));
    }

    if input == "/permissions" {
        return Ok(Some(format!(
            "permission store: {}",
            mini_code_permissions_path().display()
        )));
    }

    if input == "/skills" {
        let skills = tools.map(|t| t.get_skills()).unwrap_or(&[]);
        if skills.is_empty() {
            return Ok(Some("No skills discovered.".to_string()));
        }
        return Ok(Some(
            skills
                .iter()
                .map(|s| format!("{}  {}  [{}]", s.name, s.description, s.source))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }

    if input == "/mcp" {
        let servers = tools.map(|t| t.get_mcp_servers()).unwrap_or(&[]);
        if servers.is_empty() {
            return Ok(Some("No MCP servers configured.".to_string()));
        }
        return Ok(Some(
            servers
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
                .join("\n"),
        ));
    }

    if input == "/status" {
        let runtime = load_runtime_config(cwd)?;
        let auth = if runtime.auth_token.is_some() {
            "ANTHROPIC_AUTH_TOKEN"
        } else {
            "ANTHROPIC_API_KEY"
        };
        return Ok(Some(
            vec![
                format!("model: {}", runtime.model),
                format!("baseUrl: {}", runtime.base_url),
                format!("auth: {auth}"),
                format!("mcp servers: {}", runtime.mcp_servers.len()),
                runtime.source_summary,
            ]
            .join("\n"),
        ));
    }

    if input == "/model" {
        let runtime = load_runtime_config(cwd)?;
        return Ok(Some(format!("current model: {}", runtime.model)));
    }

    if let Some(model) = input.strip_prefix("/model ") {
        let model = model.trim();
        if model.is_empty() {
            return Ok(Some("用法: /model <model-name>".to_string()));
        }
        save_minicode_settings(MiniCodeSettings {
            model: Some(model.to_string()),
            ..MiniCodeSettings::default()
        })?;
        return Ok(Some(format!(
            "saved model={} to {}",
            model,
            mini_code_settings_path().display()
        )));
    }

    Ok(None)
}
