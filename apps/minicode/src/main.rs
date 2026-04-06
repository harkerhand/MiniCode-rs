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

    // 处理管理命令
    if let Some(command) = cli.command
        && handle_management_command(&cwd, command).await?
    {
        return Ok(());
    }

    // 初始化运行时环境
    let _ = load_runtime_config(&cwd).ok();
    let tools = Arc::new(create_default_tool_registry(&cwd).await?);

    // 处理会话选择或创建
    let (session_id, recovered_messages, initial_transcript) = if cli.resume
        && let Some(resume_id) = select_session(&cwd).await?
    {
        // 尝试加载会话数据
        match load_session(&cwd, &resume_id) {
            Ok(session) => {
                eprintln!("✨ 正在加载会话数据...\n");

                // 从会话中提取消息（直接使用 serde_json::Value）
                let recovered_messages: Vec<ChatMessage> = session.messages;

                // 将 ChatMessage 列表转换为可视化的成绩单条目
                let transcript_lines = render_recovered_messages(&recovered_messages);

                // 转换为 TranscriptLine 格式
                let transcript = transcript_lines
                    .into_iter()
                    .map(|line| TranscriptLine {
                        kind: line.kind,
                        body: line.body,
                    })
                    .collect();

                (resume_id, Some(recovered_messages), transcript)
            }
            Err(e) => {
                eprintln!("⚠️  无法加载会话: {}", e);
                eprintln!("🆕 创建新会话...\n");
                (generate_session_id(), None, vec![])
            }
        }
    } else {
        // 创建新会话
        (generate_session_id(), None, vec![])
    };

    launch_tui_app(
        &cwd,
        session_id,
        recovered_messages,
        initial_transcript,
        tools,
    )
    .await
}

/// 启动 TUI 应用的通用函数
async fn launch_tui_app(
    cwd: impl AsRef<Path>,
    session_id: String,
    recovered_messages: Option<Vec<ChatMessage>>,
    initial_transcript: Vec<TranscriptLine>,
    tools: Arc<ToolRegistry>,
) -> Result<()> {
    verify_interactive_terminal()?;

    set_active_session_context(cwd.as_ref(), session_id.clone());

    let model: Arc<dyn ModelAdapter> = if is_mock_mode() {
        Arc::new(MockModelAdapter)
    } else {
        Arc::new(AnthropicModelAdapter::new(
            tools.clone(),
            cwd.as_ref().to_path_buf(),
        ))
    };

    let permissions = PermissionManager::new(cwd.as_ref())?;

    let initial_messages = if let Some(messages) = recovered_messages {
        messages
    } else {
        let skills = tools.get_skills();
        let mcp_servers = tools.get_mcp_servers();
        vec![ChatMessage::System {
            content: build_system_prompt(
                cwd.as_ref(),
                &permissions.get_summary_text(),
                &skills,
                &mcp_servers,
            ),
        }]
    };

    set_runtime_messages(initial_messages);
    set_runtime_transcript(initial_transcript);
    init_session_permissions(permissions.clone())?;
    init_session_id(session_id)?;
    init_session_start_time(std::time::SystemTime::now())?;

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
