use clap::{Parser, Subcommand};

mod handlers;
pub use handlers::handle_management_command;

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
        about = "运行安装向导",
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
  --project  使用项目级配置（.mini-code/mcp.json）
  (默认)     使用用户级配置（~/.mini-code/mcp.json）

示例：
  minicode mcp list
  minicode mcp add my-server -- node server.js
  minicode mcp add my-server --protocol content-length -- node server.js
  minicode mcp add remote-server --protocol streamable-http --url https://example.com/mcp
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
  --protocol   通信协议（auto/content-length/newline-json/streamable-http）
  --url        远程 MCP 的 HTTP 地址（与 COMMAND 二选一）
  --header     远程请求头（KEY=VALUE，可重复指定）
  --env        环境变量（KEY=VALUE，可重复指定）
  --project    保存到项目配置而非用户配置

用法示例：
  # 基础用法
  minicode mcp add my-server -- node server.js

  # 指定协议
  minicode mcp add my-server --protocol content-length -- python server.py

  # 远程 MCP（streamable HTTP）
  minicode mcp add remote-server --protocol streamable-http --url https://example.com/mcp

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
            value_parser = ["auto", "content-length", "newline-json", "streamable-http"],
            help = "Communication protocol (default: auto-detect)"
        )]
        protocol: Option<String>,

        /// 远程 MCP 地址（streamable-http）
        #[arg(long, help = "Remote MCP endpoint URL")]
        url: Option<String>,

        /// 环境变量，格式为 KEY=VALUE（可重复）
        #[arg(
            long = "env",
            help = "Environment variable in KEY=VALUE format (repeatable)"
        )]
        env_vars: Vec<String>,

        /// 远程请求头，格式为 KEY=VALUE（可重复）
        #[arg(
            long = "header",
            help = "Remote HTTP header in KEY=VALUE format (repeatable)"
        )]
        headers: Vec<String>,

        /// 使用项目级配置而非用户级
        #[arg(long, help = "Save to project configuration")]
        project: bool,

        /// MCP 命令及参数
        #[arg(
            trailing_var_arg = true,
            required = false,
            allow_hyphen_values = true,
            help = "Command and arguments to start a local server (after --)"
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

    /// 为远程 MCP 服务器存储 bearer token
    #[command(
        about = "Store a bearer token for a remote MCP server",
        long_about = "为远程 MCP 服务器存储 bearer token，用于 HTTP 认证

用法：
  minicode mcp login <name> --token <bearer-token>"
    )]
    Login {
        /// MCP 服务名称
        #[arg(help = "Name of the MCP server")]
        name: String,

        /// Bearer token 值
        #[arg(long = "token", help = "Bearer token value")]
        token: String,
    },

    /// 清除远程 MCP 服务器的已存储 token
    #[command(
        about = "Clear a stored token for a remote MCP server",
        long_about = "删除远程 MCP 服务器的已存储 bearer token

用法：
  minicode mcp logout <name>"
    )]
    Logout {
        /// MCP 服务名称
        #[arg(help = "Name of the MCP server")]
        name: String,
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
  - ~/.mini-code/skills/       (用户级技能)
  - .mini-code/skills/        (项目级技能)
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
