pub use minicode_agent_core::*;
pub use minicode_install::*;
pub use minicode_manage::*;
pub use minicode_mock_model::*;
pub use minicode_permissions::*;
pub use minicode_tools_runtime::*;
pub use minicode_tui::*;

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use minicode_core::config::load_runtime_config;
use minicode_core::prompt::McpServerSummary;
use minicode_core::types::ModelAdapter;

/// MiniCode 命令行工具
#[derive(Debug, Parser)]
#[command(
    name = "minicode",
    version,
    about = "A Claude-powered code assistant",
    long_about = r#"MiniCode: Claude 驱动的代码助手

交互式编程环境，让 Claude 帮助您完成代码任务。

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
struct Cli {
    /// 执行的子命令
    #[command(subcommand)]
    command: Option<Command>,
}

/// 支持的子命令
#[derive(Debug, Subcommand)]
enum Command {
    /// 运行安装向导，配置 MiniCode
    #[command(
        about = "Run installation wizard",
        long_about = "交互式安装向导，帮助您配置 MiniCode 的初始设置

包括：
  - 验证 Claude API 密钥
  - 配置模型选择
  - 初始化权限系统
  - 发现和配置 MCP 服务"
    )]
    Install,

    /// 管理 MCP 服务
    #[command(
        about = "Manage MCP servers",
        long_about = "配置和管理 MCP（模型上下文协议）服务器

MCP 允许 Claude 访问外部工具、资源和数据。
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
        long_about = "发现、安装和管理 Claude 技能

技能是 Claude 可用的特定功能或知识包。

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
}

/// MCP 服务子命令
#[derive(Debug, Subcommand)]
enum McpCommand {
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

/// 技能管理子命令
#[derive(Debug, Subcommand)]
enum SkillsCommand {
    /// 列出可用的技能
    #[command(
        about = "List available skills",
        long_about = "发现并显示所有可用的 Claude 技能

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

/// 检查标准输入输出是否都连接到交互式终端。
fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// 将 CLI 命令转换为管理操作
async fn handle_management_command(cwd: &std::path::Path, cmd: Command) -> Result<bool> {
    match cmd {
        Command::Install => {
            run_install_wizard(cwd)?;
            Ok(true)
        }
        Command::Mcp { command } => handle_mcp_command(cwd, command).await,
        Command::Skills { command } => handle_skills_command(cwd, command).await,
        Command::Help => {
            print_help();
            Ok(true)
        }
    }
}

/// 打印详细的帮助信息
fn print_help() {
    let help = r#"MiniCode - 基于 Claude 的代码助手

用法：minicode [命令] [选项]

命令：
  install       运行安装向导配置 MiniCode
  mcp           管理 MCP (模型上下文协议) 服务器
  skills        发现和管理 Claude 技能
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

  # 配置作用域
  minicode mcp list --project     # 项目级 MCP
  minicode skills add ./s --project  # 项目级技能

文档：
  访问 https://github.com/harkerhand/minicode-rs 获取完整文档
"#;
    println!("{}", help);
}

/// 处理 MCP 相关命令
async fn handle_mcp_command(cwd: &std::path::Path, cmd: McpCommand) -> Result<bool> {
    match cmd {
        McpCommand::List { project } => {
            let scope = if project { "project" } else { "user" };
            minicode_manage::list_mcp_servers(cwd, scope).await
        }
        McpCommand::Add {
            name,
            protocol,
            env_vars,
            project,
            command,
        } => {
            let scope = if project { "project" } else { "user" };
            let env = minicode_manage::parse_env_pairs(&env_vars)?;
            minicode_manage::add_mcp_server(cwd, scope, name, protocol, env, command).await
        }
        McpCommand::Remove { name, project } => {
            let scope = if project { "project" } else { "user" };
            minicode_manage::remove_mcp_server(cwd, scope, name).await
        }
    }
}

/// 处理技能相关命令
async fn handle_skills_command(cwd: &std::path::Path, cmd: SkillsCommand) -> Result<bool> {
    match cmd {
        SkillsCommand::List => minicode_manage::list_skills(cwd).await,
        SkillsCommand::Add {
            path,
            name,
            project,
        } => {
            let scope = if project { "project" } else { "user" };
            minicode_manage::add_skill(cwd, scope, path, name).await
        }
        SkillsCommand::Remove { name, project } => {
            let scope = if project { "project" } else { "user" };
            minicode_manage::remove_skill(cwd, scope, name).await
        }
    }
}

/// 将日志文本按字符数截断，超出时追加省略号
fn truncate_log_text(input: &str, max_chars: usize) -> String {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    let truncated: String = chars[..max_chars].iter().collect();
    format!("{}...", truncated)
}

/// 输出 MCP 服务启动阶段的摘要日志
fn log_mcp_bootstrap(servers: &[McpServerSummary]) {
    eprintln!(
        "\x1b[1;34m[bootstrap]\x1b[0m \x1b[1mMCP servers configured:\x1b[0m {}",
        servers.len()
    );

    for server in servers {
        let status_colored = match server.status.as_str() {
            "connected" => "\x1b[32mconnected\x1b[0m",
            "error" => "\x1b[31merror\x1b[0m",
            "disabled" => "\x1b[90mdisabled\x1b[0m",
            _ => server.status.as_str(),
        };

        let mut details = vec![
            format!("status={}", status_colored),
            format!("tools={}", server.tool_count),
            format!("resources={}", server.resource_count.unwrap_or(0)),
            format!("prompts={}", server.prompt_count.unwrap_or(0)),
        ];

        if let Some(protocol) = &server.protocol {
            details.push(format!("protocol={}", protocol));
        }

        if let Some(error) = &server.error {
            details.push(format!("error={}", truncate_log_text(error, 220)));
        }

        eprintln!(
            "\x1b[1;34m[bootstrap]\x1b[0m MCP {}: {}",
            server.name,
            details.join(", ")
        );
    }
}

#[tokio::main]
/// 程序入口点，处理所有错误并以适当的退出码结束
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {:#}", err);
        std::process::exit(1);
    }
}

/// 异步主程序逻辑：解析参数、初始化运行时并启动 TUI
async fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cli = Cli::parse();

    // 处理管理命令
    if let Some(command) = cli.command
        && handle_management_command(&cwd, command).await?
    {
        return Ok(());
    }

    // 初始化运行时环境
    let runtime = load_runtime_config(&cwd).ok();
    let tools = Arc::new(create_default_tool_registry(&cwd, runtime.as_ref()).await?);
    let permissions = PermissionManager::new(cwd.clone())?;

    // 选择模型适配器
    let model: Arc<dyn ModelAdapter> = if is_mock_mode() {
        Arc::new(MockModelAdapter)
    } else {
        Arc::new(AnthropicModelAdapter::new(tools.clone(), cwd.clone()))
    };

    // 检查终端交互性
    verify_interactive_terminal()?;

    // 启动 MCP 服务并记录日志
    let mcp_servers = tools.get_mcp_servers();
    log_mcp_bootstrap(&mcp_servers);
    set_mcp_startup_logging_enabled(false);

    // 运行 TUI 应用
    run_tui_app(TuiAppArgs {
        runtime: runtime.clone(),
        tools: tools.clone(),
        model,
        cwd: cwd.clone(),
        permissions,
    })
    .await?;

    // 清理资源
    tools.dispose().await;
    Ok(())
}

/// 检查是否启用了模拟模式
fn is_mock_mode() -> bool {
    std::env::var("MINI_CODE_MODEL_MODE").ok().as_deref() == Some("mock")
}

/// 验证程序运行在交互式终端中
fn verify_interactive_terminal() -> Result<()> {
    if !is_interactive_terminal() {
        let stdin_tty = std::io::stdin().is_terminal();
        let stdout_tty = std::io::stdout().is_terminal();
        return Err(anyhow!(
            "交互模式需要在 TTY 终端中运行（stdin={}, stdout={}）",
            stdin_tty,
            stdout_tty
        ));
    }
    Ok(())
}
