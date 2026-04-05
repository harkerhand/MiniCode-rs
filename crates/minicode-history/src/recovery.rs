use std::io::BufRead;
use std::path::Path;

use anyhow::Result;
use minicode_types::{ChatMessage, TranscriptLine};

use crate::{SessionIndexEntry, find_sessions_by_prefix, load_session, load_sessions};

/// Convert ChatMessage list to visible transcript lines for session recovery
pub fn render_recovered_messages(messages: &[ChatMessage]) -> Vec<TranscriptLine> {
    let mut transcript = Vec::new();

    for msg in messages {
        match msg {
            ChatMessage::System { .. } => {}
            ChatMessage::User { content } => {
                transcript.push(TranscriptLine {
                    kind: "user".to_string(),
                    body: content.clone(),
                });
            }
            ChatMessage::Assistant { content } => {
                transcript.push(TranscriptLine {
                    kind: "assistant".to_string(),
                    body: content.clone(),
                });
            }
            ChatMessage::AssistantProgress { content } => {
                transcript.push(TranscriptLine {
                    kind: "progress".to_string(),
                    body: content.clone(),
                });
            }
            ChatMessage::AssistantToolCall {
                tool_name, input, ..
            } => {
                transcript.push(TranscriptLine {
                    kind: "tool".to_string(),
                    body: format!("{}\n{}", tool_name, input),
                });
            }
            ChatMessage::ToolResult {
                content, is_error, ..
            } => {
                transcript.push(TranscriptLine {
                    kind: if *is_error { "tool:error" } else { "tool" }.to_string(),
                    body: content.clone(),
                });
            }
        }
    }

    transcript
}

/// 根据前缀查询和加载会话，用于 history resume 命令
pub async fn resolve_and_load_session(
    cwd: impl AsRef<Path>,
    prefix: &str,
) -> Result<Option<(String, Vec<ChatMessage>, Vec<TranscriptLine>)>> {
    let matches = find_sessions_by_prefix(cwd.as_ref(), prefix)?;

    if matches.is_empty() {
        eprintln!("✗ 未找到匹配的会话: {}", prefix);
        return Ok(None);
    }

    let sessions = load_sessions(cwd.as_ref())?;

    let target_id = if matches.len() == 1 {
        matches[0].clone()
    } else {
        eprintln!("📋 找到 {} 个匹配的会话:", matches.len());

        let items: Vec<(String, String, usize, String)> = matches
            .iter()
            .filter_map(|matched_id| {
                sessions
                    .sessions
                    .iter()
                    .find(|e| &e.session_id == matched_id)
                    .map(session_item_to_tuple)
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
                "请选择要恢复的会话 (1-{}，或按 Enter 取消): ",
                matches.len()
            ),
        )? {
            Some((id, _, _, _)) => id,
            None => return Ok(None),
        }
    };

    match load_session(cwd.as_ref(), &target_id) {
        Ok(session) => {
            eprintln!("✨ 正在加载会话数据...\n");

            let recovered_messages: Vec<ChatMessage> = session.messages;

            let transcript_lines = render_recovered_messages(&recovered_messages);
            let transcript = transcript_lines
                .into_iter()
                .map(|line| TranscriptLine {
                    kind: line.kind,
                    body: line.body,
                })
                .collect();

            Ok(Some((target_id, recovered_messages, transcript)))
        }
        Err(e) => {
            eprintln!("⚠️  无法加载会话: {}", e);
            Ok(None)
        }
    }
}

fn session_item_to_tuple(entry: &SessionIndexEntry) -> (String, String, usize, String) {
    let created = entry.created_at.chars().take(19).collect::<String>();
    let model = entry.model.as_deref().unwrap_or("—").to_string();
    (entry.session_id.clone(), created, entry.turn_count, model)
}

/// 通用的交互式列表选择函数
pub fn interactive_select<T: Clone>(
    items: Vec<T>,
    format_fn: impl Fn(usize, &T) -> String,
    prompt: &str,
) -> Result<Option<T>> {
    if items.is_empty() {
        return Ok(None);
    }

    eprintln!();
    for (idx, item) in items.iter().enumerate() {
        eprintln!("{}", format_fn(idx + 1, item));
    }

    eprintln!();
    eprint!("{}", prompt);

    let stdin = std::io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;

    let line = line.trim();
    if line.is_empty() {
        eprintln!("已取消。");
        return Ok(None);
    }

    match line.parse::<usize>() {
        Ok(choice) if choice > 0 && choice <= items.len() => Ok(Some(items[choice - 1].clone())),
        _ => {
            eprintln!("✗ 无效的选择。");
            Ok(None)
        }
    }
}
