use std::{io::IsTerminal, path::Path};

use anyhow::{Result, anyhow};
use crossterm::terminal::size as terminal_size;
use minicode_core::*;

const DEFAULT_MIN_TERMINAL_WIDTH: u16 = 120;
const DEFAULT_MIN_TERMINAL_HEIGHT: u16 = 30;

/// 将日志文本按字符数截断，超出时追加省略号
pub fn truncate_log_text(input: &str, max_chars: usize) -> String {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    let truncated: String = chars[..max_chars].iter().collect();
    format!("{}...", truncated)
}

/// 输出 MCP 服务启动阶段的摘要日志
pub fn log_mcp_bootstrap(servers: &[McpServerSummary]) {
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

/// 交互式会话选择
pub async fn select_session(cwd: impl AsRef<Path>) -> Result<Option<String>> {
    let sessions = load_sessions(cwd)?;

    if sessions.sessions.is_empty() {
        eprintln!("没有找到之前的会话。");
        return Ok(None);
    }

    // 显示最近的 10 个会话
    eprintln!("\n📋 之前的会话:");
    eprintln!(
        "{:<3} {:<26} {:<6}",
        "编号", "创建时间", "回合数"
    );
    eprintln!("{}", "-".repeat(50));

    for (idx, entry) in sessions.sessions.iter().take(10).enumerate() {
        let created = entry.created_at.chars().take(19).collect::<String>();

        eprintln!(
            "{:<3} {:<26} {:<6}",
            idx + 1,
            created,
            entry.turn_count,
        );
    }

    // 获取用户输入
    eprint!(
        "\n选择会话 (1-{}，或按 Enter 取消): ",
        sessions.sessions.len().min(10)
    );
    use std::io::{self, BufRead};

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;

    let line = line.trim();
    if line.is_empty() {
        eprintln!("已取消。创建新会话。\n");
        return Ok(None);
    }

    match line.parse::<usize>() {
        Ok(choice) if choice > 0 && choice <= sessions.sessions.len().min(10) => {
            let session_id = &sessions.sessions[choice - 1].session_id;
            eprintln!("恢复会话: {}\n", session_id);
            Ok(Some(session_id.clone()))
        }
        _ => {
            eprintln!("无效的选择。创建新会话。\n");
            Ok(None)
        }
    }
}

/// 验证程序运行在交互式终端中
pub fn verify_interactive_terminal() -> Result<()> {
    if !is_interactive_terminal() {
        let stdin_tty = std::io::stdin().is_terminal();
        let stdout_tty = std::io::stdout().is_terminal();
        return Err(anyhow!(
            "交互模式需要在 TTY 终端中运行（stdin={}, stdout={}）",
            stdin_tty,
            stdout_tty
        ));
    }

    let min_width = std::env::var("MINI_CODE_MIN_TERMINAL_WIDTH")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_MIN_TERMINAL_WIDTH);
    let min_height = std::env::var("MINI_CODE_MIN_TERMINAL_HEIGHT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_MIN_TERMINAL_HEIGHT);
    let (width, height) =
        terminal_size().map_err(|err| anyhow!("无法读取终端尺寸，请在交互式终端运行: {err}"))?;
    if width < min_width {
        return Err(anyhow!(
            "终端宽度过小：当前 {} 列，最少需要 {} 列。请拉宽终端后重试，或设置 MINI_CODE_MIN_TERMINAL_WIDTH 调整阈值。",
            width,
            min_width
        ));
    }
    if height < min_height {
        return Err(anyhow!(
            "终端高度过小：当前 {} 行，最少需要 {} 行。请拉高终端后重试，或设置 MINI_CODE_MIN_TERMINAL_HEIGHT 调整阈值。",
            height,
            min_height
        ));
    }

    Ok(())
}

/// 检查标准输入输出是否都连接到交互式终端。
pub fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// 检查是否启用了模拟模式
pub fn is_mock_mode() -> bool {
    std::env::var("MINI_CODE_MODEL_MODE").ok().as_deref() == Some("mock")
}
