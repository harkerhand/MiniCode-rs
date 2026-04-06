use minicode_types::TranscriptLine;

use crate::state::{PendingApproval, ScreenState, TurnEvent};

/// 向会话转录中写入一条错误消息并更新状态。
pub(crate) fn push_error_to_session(state: &mut ScreenState, message: impl Into<String>) {
    state.transcript.push(TranscriptLine {
        kind: "tool:error".to_string(),
        body: message.into(),
    });
    state.transcript_scroll_offset = 0;
    state.status = Some("Error".to_string());
}

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
            state.active_tool = Some(tool_name.clone());
            state.status = Some(format!("Running {tool_name}..."));
            state.transcript.push(TranscriptLine {
                kind: "tool".to_string(),
                body: format!(
                    "{}\n{}",
                    tool_name,
                    summarize_tool_input(&tool_name, &input)
                ),
            });
            state.transcript_scroll_offset = 0;
            false
        }
        TurnEvent::ToolResult {
            tool_name,
            output,
            is_error,
        } => {
            state.recent_tools.push((tool_name, !is_error));
            state.transcript.push(TranscriptLine {
                kind: if is_error {
                    "tool:error".to_string()
                } else {
                    "tool".to_string()
                },
                body: output,
            });
            state.transcript_scroll_offset = 0;
            false
        }
        TurnEvent::Assistant(content) => {
            state.transcript.push(TranscriptLine {
                kind: "assistant".to_string(),
                body: content,
            });
            state.transcript_scroll_offset = 0;
            false
        }
        TurnEvent::Progress(content) => {
            state.transcript.push(TranscriptLine {
                kind: "progress".to_string(),
                body: content,
            });
            state.transcript_scroll_offset = 0;
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
        TurnEvent::ToolDone(result) => {
            state.recent_tools.push((
                state
                    .active_tool
                    .clone()
                    .unwrap_or_else(|| "tool".to_string()),
                result.ok,
            ));
            state.transcript.push(TranscriptLine {
                kind: if result.ok {
                    "tool".to_string()
                } else {
                    "tool:error".to_string()
                },
                body: result.output,
            });
            state.active_tool = None;
            state.status = None;
            false
        }
        TurnEvent::Done => true,
    }
}
