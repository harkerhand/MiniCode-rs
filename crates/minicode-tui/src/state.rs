use std::collections::HashSet;

use chrono::{DateTime, Utc};
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
    Assistant(String),
    Progress(String),
    Approval {
        request: PermissionPromptRequest,
        responder: oneshot::Sender<PermissionPromptResult>,
    },
    Done,
    ToolDone(ToolResult),
}

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
    #[allow(dead_code)]
    pub(crate) session_id: String,
    #[allow(dead_code)]
    pub(crate) session_start_time: DateTime<Utc>,
    pub(crate) turn_count: usize,
    pub(crate) context_tokens_estimate: usize,
}

impl Default for ScreenState {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_offset: 0,
            transcript_scroll_offset: 0,
            session_max_scroll_offset: 0,
            expanded_tool_entries: HashSet::new(),
            visible_tool_toggle_rows: Vec::new(),
            selected_slash_index: 0,
            status: None,
            active_tool: None,
            recent_tools: Vec::new(),
            history: Vec::new(),
            history_index: 0,
            history_draft: String::new(),
            is_busy: false,
            message_count: 0,
            pending_approval: None,
            session_id: String::new(),
            session_start_time: Utc::now(),
            turn_count: 0,
            context_tokens_estimate: 0,
        }
    }
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
}
