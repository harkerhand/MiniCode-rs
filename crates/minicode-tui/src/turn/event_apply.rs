use minicode_history::append_runtime_message;
use minicode_types::ChatMessage;

use crate::state::{PendingApproval, PendingAskUser, ScreenState, TurnEvent};

/// 为工具输入生成便于展示的简短摘要。
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
        return format!("{} path={}", tool_name, path);
    }
    if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
        return format!("{} {}", tool_name, command);
    }
    serde_json::to_string(input).unwrap_or_else(|_| "(invalid input)".to_string())
}

/// 应用单个回合事件到 UI 状态，必要时返回新消息列表。
pub(crate) fn apply_turn_event(state: &mut ScreenState, event: TurnEvent) -> bool {
    match event {
        TurnEvent::ToolStart { tool_name, input } => {
            state.stream_text.clear();
            state.stream_frozen = true;
            state.active_tool = Some(tool_name.clone());
            state.status = Some(format!("Running {tool_name}..."));
            let _ = summarize_tool_input(&tool_name, &input);
            false
        }
        TurnEvent::ToolResult {
            tool_name,
            output,
            is_error,
        } => {
            state.recent_tools.push((tool_name, !is_error));
            let _ = output;
            false
        }
        TurnEvent::Assistant(content) => {
            state.stream_text.clear(); // 完整消息已入库后再到这里，清除流式残影
            state.stream_frozen = true;
            let _ = content;
            false
        }
        TurnEvent::Progress(content) => {
            let _ = content;
            false
        }
        TurnEvent::Approval { request, responder } => {
            state.pending_approval = Some(PendingApproval {
                request,
                responder: Some(responder),
                selected_index: 0,
                awaiting_feedback: false,
                feedback: String::new(),
            });
            state.status = Some("Approval required...".to_string());
            false
        }
        TurnEvent::Status(text) => {
            state.status = Some(text);
            false
        }
        TurnEvent::AskUserPrompt { question, options } => {
            state.pending_ask_user = Some(PendingAskUser {
                question,
                options,
                selected_index: 0,
            });
            state.status = Some("Ask user...".to_string());
            false
        }
        TurnEvent::StreamDelta(delta, is_final) => {
            if state.stream_frozen {
                return false;
            }
            if is_final {
                // final 只表示流结束，不在这里清空，避免“先回退再出现最终文本”
            } else {
                // 兼容两类 provider：
                // 1) delta 增量；2) cumulative 全量片段。
                if state.stream_text.is_empty() {
                    state.stream_text.push_str(&delta);
                } else if delta.starts_with(&state.stream_text) {
                    state.stream_text = delta;
                } else {
                    state.stream_text.push_str(&delta);
                }
            }
            false
        }
        TurnEvent::ToolDone(result) => {
            state.recent_tools.push((
                state
                    .active_tool
                    .clone()
                    .unwrap_or_else(|| "tool".to_string()),
                result.ok,
            ));
            let kind = if result.ok {
                "command:result"
            } else {
                "command:error"
            };
            append_runtime_message(ChatMessage::runtime_display(kind, result.output));
            state.active_tool = None;
            state.status = None;
            false
        }
        TurnEvent::Done => {
            state.stream_text.clear();
            state.stream_frozen = true;
            true
        }
    }
}
