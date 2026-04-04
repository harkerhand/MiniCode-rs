use std::{path::Path, sync::Arc};

use anyhow::Result;
use clap::{Parser, Subcommand};
use minicode_core::*;

use crate::launch_tui_app;

/// MiniCode 命令行工具
#[derive(Debug, Parser)]
#[command(
    name = "minicode",
    version,
    about = "A code assistant",
    long_about = r#"MiniCode 驱动的代码助手

交互式编程环境，让 MiniCode 帮助您完成代码任务。

使用示例：
  minicode                    # 启动交互式 TUI 环境
  minicode install            # 运行安装向导
  minicode mcp list           # 列出已配置的 MCP 服务
  minicode mcp add claude -- npx @anthropic-ai/sdk
  minicode skills list        # 列出可用技能
  minicode skills add ./my-skill --name my-skill

更多信息：
  minicode --help
  minicode mcp --help
  minicode skills --help
  minicode install --help
"#,
    disable_help_subcommand = true,
    propagate_version = true
)]
pub(crate) struct Cli {
    /// 恢复之前的会话
    #[arg(long, help = "Resume a previous session")]
    pub(crate) resume: bool,

    /// 执行的子命令
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// 支持的子命令
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// 运行安装向导，配置 MiniCode
    #[command(
        about = "Run installation wizard",
        long_about = "交互式安装向导，帮助您配置 MiniCode 的初始设置

包括：
  - 验证 API 密钥
  - 配置模型选择
  - 初始化权限系统
  - 发现和配置 MCP 服务"
    )]
    Install,

    /// 管理 MCP 服务
    #[command(
        about = "Manage MCP servers",
        long_about = "配置和管理 MCP（模型上下文协议）服务器

MCP 允许 MiniCode 访问外部工具、资源和数据。
使用 mcp 命令可以列出、添加和移除服务器。

配置作用域：
  --project  使用项目级配置（.minicode/mcp.json）
  (默认)     使用用户级配置（~/.minicode/mcp.json）

示例：
  minicode mcp list
  minicode mcp add my-server -- node server.js
  minicode mcp add my-server --protocol content-length -- node server.js
  minicode mcp add my-server --env API_KEY=xxx --env DEBUG=1 -- node server.js"
    )]
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },

    /// 管理技能
    #[command(
        about = "Manage skills",
        long_about = "发现、安装和管理 MiniCode 技能

技能是 MiniCode 可用的特定功能或知识包。

配置作用域：
  --project  使用项目级配置
  (默认)     使用用户级配置

示例：
  minicode skills list
  minicode skills add /path/to/skill
  minicode skills add ./my-skill --name custom-name --project
  minicode skills remove my-skill"
    )]
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },

    /// 显示帮助信息
    #[command(about = "Show help")]
    Help,

    /// 管理会话历史
    #[command(
        about = "Manage session history",
        long_about = "查看、恢复和删除会话历史记录

会话记录包含完整的对话、工具调用和模型交互。

示例：
  minicode history list           # 列出所有会话
  minicode history list claude-3  # 按 model 过滤
  minicode history rm <session_id>  # 删除会话
  minicode history resume <session_id>  # 恢复会话"
    )]
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
}

/// MCP 服务子命令
#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// 列出已配置的 MCP 服务
    #[command(
        about = "List configured MCP servers",
        long_about = "显示所有已配置的 MCP 服务器及其详细信息

包括：
  - 服务器名称
  - 启动命令
  - 通信协议
  - 工具和资源数量

用法：
  minicode mcp list          # 列出用户级服务器
  minicode mcp list --project  # 列出项目级服务器"
    )]
    List {
        /// 使用项目级配置而非用户级
        #[arg(long, help = "Show project-level servers instead of user-level")]
        project: bool,
    },

    /// 添加新的 MCP 服务
    #[command(
        about = "Add a new MCP server",
        long_about = "注册一个新的 MCP 服务器

必需参数：
  <NAME>       服务器的唯一名称
  -- <COMMAND> 启动服务器的命令（在 -- 后指定）

可选标志：
  --protocol   通信协议（auto/content-length/newline-json）
  --env        环境变量（KEY=VALUE，可重复指定）
  --project    保存到项目配置而非用户配置

用法示例：
  # 基础用法
  minicode mcp add my-server -- node server.js

  # 指定协议
  minicode mcp add my-server --protocol content-length -- python server.py

  # 添加环境变量
  minicode mcp add my-server --env API_KEY=xxx --env DEBUG=1 -- node server.js

  # 项目级配置
  minicode mcp add my-server -- node server.js --project"
    )]
    Add {
        /// MCP 服务名称
        #[arg(help = "Unique name for this server")]
        name: String,

        /// 通信协议
        #[arg(
            long,
            value_parser = ["auto", "content-length", "newline-json"],
            help = "Communication protocol (default: auto-detect)"
        )]
        protocol: Option<String>,

        /// 环境变量，格式为 KEY=VALUE（可重复）
        #[arg(
            long = "env",
            help = "Environment variable in KEY=VALUE format (repeatable)"
        )]
        env_vars: Vec<String>,

        /// 使用项目级配置而非用户级
        #[arg(long, help = "Save to project configuration")]
        project: bool,

        /// MCP 命令及参数
        #[arg(
            trailing_var_arg = true,
            required = true,
            allow_hyphen_values = true,
            help = "Command and arguments to start the server (after --)"
        )]
        command: Vec<String>,
    },

    /// 移除 MCP 服务
    #[command(
        about = "Remove an MCP server",
        long_about = "从配置中删除已注册的 MCP 服务器

用法：
  minicode mcp remove my-server          # 从用户配置删除
  minicode mcp remove my-server --project  # 从项目配置删除"
    )]
    Remove {
        /// MCP 服务名称
        #[arg(help = "Name of the server to remove")]
        name: String,

        /// 使用项目级配置而非用户级
        #[arg(long, help = "Remove from project configuration")]
        project: bool,
    },
}

/// 会话历史管理子命令
#[derive(Debug, Subcommand)]
pub(crate) enum HistoryCommand {
    /// 列出会话历史
    #[command(
        about = "List all sessions",
        long_about = "显示所有会话及其详细信息

列显内容：
  - Session ID (前16个字符)
  - 创建时间 (ISO 8601 格式)
  - 结束时间
  - 对话轮数
  - 使用的模型
  - 状态 (active/completed)

用法：
  minicode history list           # 列出所有会话
  minicode history list claude-3  # 按模型名称过滤
  minicode history list sess_abc  # 按 session_id 过滤"
    )]
    List {
        /// 可选的过滤条件（会话ID或模型名称）
        #[arg(help = "Optional filter by session_id or model")]
        filter: Option<String>,
    },

    /// 删除会话
    #[command(
        about = "Delete a session",
        long_about = "删除指定的会话及其所有数据

注意：此操作不可恢复。删除的会话包括：
  - 对话历史
  - 工具调用记录
  - 会话元数据
  - 输入历史

用法：
  minicode history rm <session_id>"
    )]
    Rm {
        /// 要删除的会话 ID
        #[arg(help = "Session ID to delete")]
        session_id: String,
    },

    /// 恢复会话
    #[command(
        about = "Resume a specific session",
        long_about = "启动 MiniCode 并恢复指定的会话

这等同于运行 'minicode --resume' 然后选择对应的会话。

用法：
  minicode history resume <session_id>"
    )]
    Resume {
        /// 要恢复的会话 ID
        #[arg(help = "Session ID to resume")]
        session_id: String,
    },
}

/// 技能管理子命令
#[derive(Debug, Subcommand)]
pub(crate) enum SkillsCommand {
    /// 列出可用的技能
    #[command(
        about = "List available skills",
        long_about = "发现并显示所有可用的 MiniCode 技能

技能被自动发现于以下位置：
  - ~/.minicode/skills/       (用户级技能)
  - .minicode/skills/        (项目级技能)
  - 其他配置的技能目录

每个技能显示：
  - 名称和描述
  - 安装位置"
    )]
    List,

    /// 安装技能
    #[command(
        about = "Install a skill from path",
        long_about = "从本地路径安装或复制技能到 MiniCode

参数：
  <PATH>   技能文件或目录的路径

可选标志：
  --name      自定义技能名称（默认使用目录名）
  --project   安装到项目级位置而非用户级

用法示例：
  # 从目录安装技能
  minicode skills add ./my-skill

  # 指定自定义名称
  minicode skills add ./my-skill --name awesome-skill

  # 安装到项目级
  minicode skills add ./my-skill --project

  # 从远程克隆的技能
  minicode skills add ~/Downloads/skill-repo --name imported-skill"
    )]
    Add {
        /// 技能文件或目录路径
        #[arg(help = "Path to skill file or directory")]
        path: String,

        /// 自定义技能名称
        #[arg(long, help = "Custom name for the skill (defaults to directory name)")]
        name: Option<String>,

        /// 使用项目级配置而非用户级
        #[arg(long, help = "Install to project location")]
        project: bool,
    },

    /// 移除技能
    #[command(
        about = "Remove an installed skill",
        long_about = "从配置中删除已安装的技能

用法：
  minicode skills remove my-skill          # 移除用户级技能
  minicode skills remove my-skill --project  # 移除项目级技能

注意：只删除管理的技能副本，原始源文件保持不变"
    )]
    Remove {
        /// 技能名称
        #[arg(help = "Name of the skill to remove")]
        name: String,

        /// 使用项目级配置而非用户级
        #[arg(long, help = "Remove from project location")]
        project: bool,
    },
}

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
        McpCommand::List { project } => {
            let scope = if project { "project" } else { "user" };
            list_mcp_servers(cwd.as_ref(), scope).await
        }
        McpCommand::Add {
            name,
            protocol,
            env_vars,
            project,
            command,
        } => {
            let scope = if project { "project" } else { "user" };
            let env = parse_env_pairs(&env_vars)?;
            add_mcp_server(cwd.as_ref(), scope, name, protocol, env, command).await
        }
        McpCommand::Remove { name, project } => {
            let scope = if project { "project" } else { "user" };
            remove_mcp_server(cwd.as_ref(), scope, name).await
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
        } => {
            let scope = if project { "project" } else { "user" };
            add_skill(cwd.as_ref(), scope, path, name).await
        }
        SkillsCommand::Remove { name, project } => {
            let scope = if project { "project" } else { "user" };
            remove_skill(cwd.as_ref(), scope, name).await
        }
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
            // Find sessions matching the prefix
            let matches = find_sessions_by_prefix(cwd.as_ref(), &session_id)?;

            if matches.is_empty() {
                eprintln!("✗ 未找到匹配的会话: {}", session_id);
                return Ok(true);
            }

            let target_id = if matches.len() == 1 {
                // Single match - delete directly
                matches[0].clone()
            } else {
                // Multiple matches - interactive selection
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
