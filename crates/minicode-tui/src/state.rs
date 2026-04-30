use std::collections::HashSet;

use minicode_agent_core::AgentTurnCallbacks;
use minicode_permissions::{PermissionPromptRequest, PermissionPromptResult};
use minicode_tool::ToolResult;
use tokio::sync::{mpsc, oneshot};

pub(crate) struct PendingApproval {
    pub(crate) request: PermissionPromptRequest,
    pub(crate) responder: Option<oneshot::Sender<PermissionPromptResult>>,
    pub(crate) selected_index: usize,
    pub(crate) awaiting_feedback: bool,
    pub(crate) feedback: String,
}

pub(crate) struct PendingAskUser {
    pub(crate) question: String,
    pub(crate) options: Vec<String>,
    pub(crate) selected_index: usize,
}

pub(crate) enum TurnEvent {
    ToolStart {
        tool_name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_name: String,
        output: String,
        is_error: bool,
    },
    /// 流式文本增量（delta, is_final）
    StreamDelta(String, bool),
    /// 更新状态栏文字
    Status(String),
    AskUserPrompt {
        question: String,
        options: Vec<String>,
    },
    Assistant(String),
    Progress(String),
    Approval {
        request: PermissionPromptRequest,
        responder: oneshot::Sender<PermissionPromptResult>,
    },
    Done,
    ToolDone(ToolResult),
}

#[derive(Default)]
pub(crate) struct ScreenState {
    pub(crate) input: String,
    pub(crate) cursor_offset: usize,
    pub(crate) transcript_scroll_offset: usize,
    pub(crate) session_max_scroll_offset: usize,
    pub(crate) expanded_tool_entries: HashSet<usize>,
    pub(crate) visible_tool_toggle_rows: Vec<(u16, usize)>,
    pub(crate) selected_slash_index: usize,
    pub(crate) status: Option<String>,
    pub(crate) active_tool: Option<String>,
    pub(crate) recent_tools: Vec<(String, bool)>,
    pub(crate) history: Vec<String>,
    pub(crate) history_index: usize,
    pub(crate) history_draft: String,
    pub(crate) is_busy: bool,
    pub(crate) message_count: usize,
    pub(crate) pending_approval: Option<PendingApproval>,
    pub(crate) pending_ask_user: Option<PendingAskUser>,
    pub(crate) turn_count: usize,
    pub(crate) context_tokens_estimate: usize,
    pub(crate) queued_busy_inputs: Vec<String>,
    /// 流式输出累积文本
    pub(crate) stream_text: String,
    /// 为防止乱序回退：最终消息落地后冻结流式增量，忽略迟到 chunk
    pub(crate) stream_frozen: bool,
}

pub(crate) struct ChannelCallbacks {
    pub(crate) tx: mpsc::UnboundedSender<TurnEvent>,
}

impl AgentTurnCallbacks for ChannelCallbacks {
    /// 通知 UI 当前开始执行某个工具。
    fn on_tool_start(&mut self, tool_name: &str, input: &serde_json::Value) {
        let _ = self.tx.send(TurnEvent::ToolStart {
            tool_name: tool_name.to_string(),
            input: input.clone(),
        });
    }

    /// 通知 UI 工具执行完成及其结果。
    fn on_tool_result(&mut self, tool_name: &str, output: &str, is_error: bool) {
        let _ = self.tx.send(TurnEvent::ToolResult {
            tool_name: tool_name.to_string(),
            output: output.to_string(),
            is_error,
        });
    }

    /// 将助手最终消息转发到事件通道。
    fn on_assistant_message(&mut self, content: &str) {
        let _ = self.tx.send(TurnEvent::Assistant(content.to_string()));
    }

    /// 将助手进度消息转发到事件通道。
    fn on_progress_message(&mut self, content: &str) {
        let _ = self.tx.send(TurnEvent::Progress(content.to_string()));
    }

    /// 上下文压缩开始：更新状态栏。
    fn on_compact_start(&mut self) {
        let _ = self
            .tx
            .send(TurnEvent::Status("Compacting context...".to_string()));
    }

    /// 上下文被压缩时通知 UI。
    fn on_compact(&mut self, _summary: &str) {
        let _ = self
            .tx
            .send(TurnEvent::Progress("上下文已自动压缩以节省 token。".to_string()));
        let _ = self
            .tx
            .send(TurnEvent::Status("Thinking...".to_string()));
    }

    /// 流式输出文本增量。
    fn on_stream_chunk(&mut self, delta: &str, is_final: bool) {
        let _ = self
            .tx
            .send(TurnEvent::StreamDelta(delta.to_string(), is_final));
    }

    fn on_ask_user_prompt(&mut self, question: &str, options: &[String]) {
        let _ = self.tx.send(TurnEvent::AskUserPrompt {
            question: question.to_string(),
            options: options.to_vec(),
        });
    }
}
