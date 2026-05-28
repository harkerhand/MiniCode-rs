use anyhow::Result;
use minicode_config::{
    mini_code_mcp_path, mini_code_permissions_path, mini_code_settings_path,
    modify_runtime_config, runtime_config, save_minicode_settings,
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
                modify_runtime_config(runtime.clone());
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
    SlashCommand {
        prefix: "/resume",
        usage: "/resume [session-id]",
        description: "恢复之前的会话。",
        handler: |input| {
            let session_id = input.trim_start_matches("/resume").trim().to_string();
            Box::pin(async move {
                let cwd = minicode_config::runtime_store().cwd.clone();

                if session_id.is_empty() {
                    // 交互式选择
                    let sessions = minicode_history::load_sessions(&cwd)?;
                    if sessions.sessions.is_empty() {
                        return Ok("没有可恢复的会话。".to_string());
                    }

                    let items: Vec<(String, String, usize)> = sessions
                        .sessions
                        .iter()
                        .map(|s| {
                            let created = s.created_at.chars().take(19).collect::<String>();
                            (s.session_id.clone(), created, s.turn_count)
                        })
                        .collect();

                    let selected = minicode_history::interactive_select(
                        items,
                        |idx, (id, created, turns)| {
                            format!(
                                "{:<2} {:<18} {:<20} {:<6}",
                                idx,
                                &id[..id.len().min(16)],
                                created,
                                turns,
                            )
                        },
                        "选择要恢复的会话: ",
                    )?;

                    if let Some((id, _, _)) = selected {
                        // 加载会话
                        let messages = minicode_history::load_session_messages(&cwd, &id)?;
                        minicode_history::clear_runtime_messages();
                        for msg in messages {
                            minicode_history::append_runtime_message(msg);
                        }
                        return Ok(format!("已恢复会话: {}", id));
                    }

                    return Ok("已取消。".to_string());
                }

                // 直接恢复指定会话
                let messages = minicode_history::load_session_messages(&cwd, &session_id)?;
                minicode_history::clear_runtime_messages();
                for msg in messages {
                    minicode_history::append_runtime_message(msg);
                }
                Ok(format!("已恢复会话: {}", session_id))
            })
        },
    },
    SlashCommand {
        prefix: "/rename",
        usage: "/rename <new-name>",
        description: "重命名当前会话。",
        handler: |input| {
            let new_name = input.trim_start_matches("/rename").trim().to_string();
            Box::pin(async move {
                if new_name.is_empty() {
                    return Err(anyhow::anyhow!("请提供新名称。用法: /rename <new-name>"));
                }

                let cwd = minicode_config::runtime_store().cwd.clone();
                let session_id = minicode_config::runtime_store().session_id.clone();

                minicode_history::rename_session(&cwd, &session_id, &new_name)?;

                Ok(format!("会话已重命名为: {}", new_name))
            })
        },
    },
    SlashCommand {
        prefix: "/fork",
        usage: "/fork [session-id]",
        description: "Fork 当前或指定会话为新会话。",
        handler: |input| {
            let session_id = input.trim_start_matches("/fork").trim().to_string();
            Box::pin(async move {
                let cwd = minicode_config::runtime_store().cwd.clone();

                let source_id = if session_id.is_empty() {
                    minicode_config::runtime_store().session_id.clone()
                } else {
                    session_id
                };

                // 加载源会话
                let source_messages =
                    minicode_history::load_session_messages(&cwd, &source_id)?;

                // 创建新会话
                let new_session_id = minicode_history::generate_session_id();
                let mut new_messages = source_messages.clone();

                // 添加 fork 标记
                new_messages.push(minicode_types::ChatMessage::Runtime {
                    kind: "fork".to_string(),
                    content: format!("Forked from session: {}", source_id),
                    flags: minicode_types::MessageFlags::recorded_context_display(),
                });

                // 保存新会话
                minicode_history::save_session_messages(&cwd, &new_session_id, &new_messages)?;

                // 切换到新会话
                minicode_history::clear_runtime_messages();
                for msg in new_messages {
                    minicode_history::append_runtime_message(msg);
                }

                Ok(format!(
                    "已 fork 会话 {} -> {}",
                    source_id, new_session_id
                ))
            })
        },
    },
    SlashCommand {
        prefix: "/init",
        usage: "/init",
        description: "初始化项目配置（创建 .mini-code/ 目录和 MINI.md）。",
        handler: |_| {
            Box::pin(async move {
                let cwd = minicode_config::runtime_store().cwd.clone();
                let mini_code_dir = cwd.join(".mini-code");
                let skills_dir = mini_code_dir.join("skills");
                let rules_dir = mini_code_dir.join("rules");
                let mini_md = cwd.join("MINI.md");
                let gitignore = cwd.join(".gitignore");

                // 创建目录
                std::fs::create_dir_all(&mini_code_dir)?;
                std::fs::create_dir_all(&skills_dir)?;
                std::fs::create_dir_all(&rules_dir)?;

                // 创建 MINI.md（如果不存在）
                if !mini_md.exists() {
                    let template = detect_project_template(&cwd);
                    std::fs::write(&mini_md, template)?;
                }

                // 更新 .gitignore
                if gitignore.exists() {
                    let content = std::fs::read_to_string(&gitignore)?;
                    if !content.contains(".mini-code/sessions") {
                        std::fs::write(
                            &gitignore,
                            format!(
                                "{}\n\n# MiniCode sessions\n.mini-code/sessions/\n",
                                content
                            ),
                        )?;
                    }
                } else {
                    std::fs::write(
                        &gitignore,
                        "# MiniCode sessions\n.mini-code/sessions/\n",
                    )?;
                }

                Ok(format!(
                    "项目已初始化：\n  - {}\n  - {}\n  - {}\n  - {}",
                    mini_code_dir.display(),
                    skills_dir.display(),
                    rules_dir.display(),
                    mini_md.display()
                ))
            })
        },
    },
    SlashCommand {
        prefix: "/memory",
        usage: "/memory",
        description: "显示已加载的指令文件。",
        handler: |_| {
            Box::pin(async move {
                let cwd = minicode_config::runtime_store().cwd.clone();
                let home = dirs::home_dir().unwrap_or_default();

                Ok(minicode_prompt::render_memory_report(&cwd, &home))
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

/// 检测项目技术栈并生成模板
fn detect_project_template(cwd: &std::path::Path) -> String {
    let mut template = String::from("# Project Instructions\n\n");

    // 检测技术栈
    let mut stack = Vec::new();

    if cwd.join("package.json").exists() {
        stack.push("Node.js/JavaScript");
    }
    if cwd.join("tsconfig.json").exists() {
        stack.push("TypeScript");
    }
    if cwd.join("Cargo.toml").exists() {
        stack.push("Rust");
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        stack.push("Python");
    }
    if cwd.join("go.mod").exists() {
        stack.push("Go");
    }
    if cwd.join("pom.xml").exists() || cwd.join("build.gradle").exists() {
        stack.push("Java");
    }

    if !stack.is_empty() {
        template.push_str(&format!("Detected stack: {}\n\n", stack.join(", ")));
    }

    template.push_str("## Guidelines\n\n");
    template.push_str("- Follow existing code conventions\n");
    template.push_str("- Write tests for new features\n");
    template.push_str("- Keep changes minimal and focused\n");

    template
}
