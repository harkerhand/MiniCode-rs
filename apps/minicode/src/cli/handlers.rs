use std::{path::Path, sync::Arc};

use anyhow::Result;
use minicode_core::*;

use crate::cli::{Command, HistoryCommand, McpCommand, SkillsCommand};
use crate::launch_tui_app;

/// 将 CLI 命令转换为管理操作
pub async fn handle_management_command(cwd: impl AsRef<Path>, cmd: Command) -> Result<bool> {
    match cmd {
        Command::Install => {
            run_install_wizard(cwd)?;
            Ok(true)
        }
        Command::Mcp { command } => handle_mcp_command(cwd, command).await,
        Command::Skills { command } => handle_skills_command(cwd, command).await,
        Command::History { command } => handle_history_command(cwd, command).await,
        Command::Help => {
            print_help();
            Ok(true)
        }
    }
}

/// 打印详细的帮助信息
fn print_help() {
    let help = r#"MiniCode 代码助手

用法：minicode [命令] [选项]

命令：
  install       运行安装向导配置 MiniCode
  mcp           管理 MCP (模型上下文协议) 服务器
  skills        发现和管理 MiniCode 技能
  history       管理会话历史记录
  help          显示此帮助信息

无命令运行：
  minicode      启动交互式 TUI 环境

快速开始：
  1. minicode install          # 首次配置
  2. minicode mcp list         # 查看 MCP 服务
  3. minicode                  # 启动编程环境

获取更多信息：
  minicode mcp --help          # MCP 命令帮助
  minicode skills --help       # 技能命令帮助
  minicode history --help      # 历史记录命令帮助
  minicode install --help      # 安装向导帮助
  minicode --version           # 显示版本

示例：
  # MCP 管理
  minicode mcp list
  minicode mcp add my-server -- node server.js
  minicode mcp add remote-server --protocol streamable-http --url https://example.com/mcp
  minicode mcp remove my-server

  # 技能管理
  minicode skills list
  minicode skills add ./my-skill --name my-skill
  minicode skills remove my-skill

  # 会话历史
  minicode history list
  minicode history list claude-3
  minicode history rm <session_id>
  minicode history resume <session_id>

  # 配置作用域
  minicode mcp list --project     # 项目级 MCP
  minicode skills add ./s --project  # 项目级技能

文档：
  访问 https://github.com/harkerhand/minicode-rs 获取完整文档
"#;
    println!("{}", help);
}

/// 处理 MCP 相关命令
async fn handle_mcp_command(cwd: impl AsRef<Path>, cmd: McpCommand) -> Result<bool> {
    match cmd {
        McpCommand::List { project } => list_mcp_servers(cwd.as_ref(), project).await,
        McpCommand::Add {
            name,
            protocol,
            url,
            env_vars,
            headers,
            project,
            command,
        } => {
            let env = parse_env_pairs(&env_vars)?;
            let header_map = parse_env_pairs(&headers)?;
            add_mcp_server(
                cwd.as_ref(),
                project,
                name,
                McpServerConfig::new(protocol, env, url, header_map, command)?,
            )
            .await
        }
        McpCommand::Remove { name, project } => {
            remove_mcp_server(cwd.as_ref(), project, name).await
        }
    }
}

/// 处理技能相关命令
async fn handle_skills_command(cwd: impl AsRef<Path>, cmd: SkillsCommand) -> Result<bool> {
    match cmd {
        SkillsCommand::List => list_skills(cwd.as_ref()).await,
        SkillsCommand::Add {
            path,
            name,
            project,
        } => add_skill(cwd.as_ref(), project, path, name).await,
        SkillsCommand::Remove { name, project } => remove_skill(cwd.as_ref(), project, name).await,
    }
}

/// 处理会话历史相关命令
async fn handle_history_command(cwd: impl AsRef<Path>, cmd: HistoryCommand) -> Result<bool> {
    match cmd {
        HistoryCommand::List { filter } => {
            let output = list_sessions_formatted(cwd.as_ref(), filter.as_deref())?;
            println!("{}", output);
            Ok(true)
        }
        HistoryCommand::Rm { session_id } => {
            let matches = find_sessions_by_prefix(cwd.as_ref(), &session_id)?;

            if matches.is_empty() {
                eprintln!("✗ 未找到匹配的会话: {}", session_id);
                return Ok(true);
            }

            let target_id = if matches.len() == 1 {
                matches[0].clone()
            } else {
                eprintln!("📋 找到 {} 个匹配的会话:", matches.len());

                let sessions = load_sessions(cwd.as_ref())?;
                let items: Vec<(String, String, usize, String)> = matches
                    .iter()
                    .filter_map(|matched_id| {
                        sessions
                            .sessions
                            .iter()
                            .find(|e| &e.session_id == matched_id)
                            .map(|entry| {
                                let created = entry.created_at.chars().take(19).collect::<String>();
                                let model = entry.model.as_deref().unwrap_or("—").to_string();
                                (matched_id.clone(), created, entry.turn_count, model)
                            })
                    })
                    .collect();

                match interactive_select(
                    items,
                    |idx, (id, created, turns, model)| {
                        format!(
                            "{:<2} {:<18} {:<20} {:<6} {:<30}",
                            idx,
                            &id[..id.len().min(16)],
                            created,
                            turns,
                            model
                        )
                    },
                    &format!(
                        "请选择要删除的会话 (1-{}，或按 Enter 取消): ",
                        matches.len()
                    ),
                )? {
                    Some((id, _, _, _)) => id,
                    None => return Ok(true),
                }
            };

            delete_session(cwd.as_ref(), &target_id)?;
            println!("✓ 会话已删除: {}", &target_id[..target_id.len().min(16)]);
            Ok(true)
        }
        HistoryCommand::Resume { session_id } => {
            match resolve_and_load_session(cwd.as_ref(), &session_id).await? {
                Some((session_id, recovered_messages, initial_transcript)) => {
                    let runtime = load_runtime_config(cwd.as_ref()).ok();
                    let tools = Arc::new(
                        create_default_tool_registry(cwd.as_ref(), runtime.as_ref()).await?,
                    );

                    launch_tui_app(
                        cwd.as_ref(),
                        session_id,
                        Some(recovered_messages),
                        initial_transcript,
                        runtime,
                        tools,
                    )
                    .await?;

                    Ok(true)
                }
                None => Ok(true),
            }
        }
    }
}
