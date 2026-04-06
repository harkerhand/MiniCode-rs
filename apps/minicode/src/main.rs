use std::{path::Path, sync::Arc};

use anyhow::Result;

use clap::Parser;
use minicode_core::*;
mod cli;
use cli::*;
mod utils;
use utils::*;

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

    // 尽早确定会话并初始化 RuntimeStore
    let mut recovered_messages: Option<Vec<ChatMessage>> = None;
    let session_id = match &cli.command {
        Some(Command::History {
            command: HistoryCommand::Resume { session_id },
        }) => match resolve_and_load_session(&cwd, session_id).await? {
            Some((resolved_session_id, recovered)) => {
                recovered_messages = Some(recovered);
                resolved_session_id
            }
            None => return Ok(()),
        },
        Some(_) => generate_session_id(),
        None => {
            if cli.resume
                && let Some(resume_id) = select_session(&cwd).await?
            {
                match load_session(&cwd, &resume_id) {
                    Ok(session) => {
                        eprintln!("✨ 正在加载会话数据...\n");
                        recovered_messages = Some(session.messages);
                        resume_id
                    }
                    Err(e) => {
                        eprintln!("⚠️  无法加载会话: {}", e);
                        eprintln!("🆕 创建新会话...\n");
                        generate_session_id()
                    }
                }
            } else {
                generate_session_id()
            }
        }
    };
    init_runtime_store(&cwd, session_id);
    if let Some(messages) = recovered_messages {
        set_runtime_messages(messages);
    }

    // 处理管理命令（history resume 由上面的预处理直接进入 TUI）
    if let Some(command) = cli.command {
        if matches!(
            command,
            Command::History {
                command: HistoryCommand::Resume { .. }
            }
        ) {
            let _ = load_runtime_config().ok();
            let tools = Arc::new(create_default_tool_registry(&cwd).await?);
            return launch_tui_app(&cwd, tools).await;
        }
        if handle_management_command(&cwd, command).await? {
            return Ok(());
        }
    }

    // 初始化运行时环境
    let _ = load_runtime_config().ok();
    let tools = Arc::new(create_default_tool_registry(&cwd).await?);
    launch_tui_app(&cwd, tools).await
}

/// 启动 TUI 应用的通用函数
async fn launch_tui_app(cwd: impl AsRef<Path>, tools: Arc<ToolRegistry>) -> Result<()> {
    verify_interactive_terminal()?;

    let model: Arc<dyn ModelAdapter> = if is_mock_mode() {
        Arc::new(MockModelAdapter)
    } else {
        Arc::new(AnthropicModelAdapter::new(tools.clone()))
    };

    let permissions = PermissionManager::new(cwd.as_ref())?;

    if runtime_messages().is_empty() {
        let skills = tools.get_skills();
        let mcp_servers = tools.get_mcp_servers();
        set_runtime_messages(vec![ChatMessage::System {
            content: build_system_prompt(
                cwd.as_ref(),
                &permissions.get_summary_text(),
                &skills,
                &mcp_servers,
            ),
        }]);
    }
    init_session_permissions(permissions.clone())?;

    let mcp_servers = tools.get_mcp_servers();
    log_mcp_bootstrap(&mcp_servers);
    set_mcp_startup_logging_enabled(false);

    run_tui_app(TuiAppArgs {
        tools: tools.clone(),
        model,
        cwd: cwd.as_ref().into(),
    })
    .await?;

    tools.dispose().await;
    println!("👋 再见！");
    Ok(())
}
