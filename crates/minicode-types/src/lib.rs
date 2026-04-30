use std::{
    fmt::Display,
    sync::{Arc, OnceLock},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerSummary {
    pub name: String,
    pub command: String,
    pub status: String,
    pub tool_count: usize,
    pub error: Option<String>,
    pub protocol: Option<String>,
    pub resource_count: Option<usize>,
    pub prompt_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum ChatMessage {
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "minicode")]
    Minicode { content: String },
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant { content: String },
    #[serde(rename = "assistant_progress")]
    AssistantProgress { content: String },
    #[serde(rename = "context_summary")]
    ContextSummary { content: String },
    #[serde(rename = "assistant_tool_call")]
    AssistantToolCall {
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    #[serde(rename = "runtime")]
    Runtime {
        kind: String,
        content: String,
        flags: MessageFlags,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFlags(u8);

impl MessageFlags {
    pub const RECORD: u8 = 1 << 0;
    pub const CONTEXT: u8 = 1 << 1;
    pub const DISPLAY: u8 = 1 << 2;

    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn recorded() -> Self {
        Self(Self::RECORD)
    }

    pub const fn context() -> Self {
        Self(Self::CONTEXT)
    }

    pub const fn display() -> Self {
        Self(Self::DISPLAY)
    }

    pub const fn recorded_context_display() -> Self {
        Self(Self::RECORD | Self::CONTEXT | Self::DISPLAY)
    }

    pub const fn contains(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

impl ChatMessage {
    pub fn runtime_display(kind: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Runtime {
            kind: kind.into(),
            content: content.into(),
            flags: MessageFlags::display(),
        }
    }

    pub fn flags(&self) -> MessageFlags {
        match self {
            ChatMessage::System { .. } => MessageFlags::new(MessageFlags::CONTEXT),
            ChatMessage::Minicode { .. } | ChatMessage::ContextSummary { .. } => {
                MessageFlags::new(MessageFlags::RECORD | MessageFlags::CONTEXT)
            }
            ChatMessage::User { .. }
            | ChatMessage::Assistant { .. }
            | ChatMessage::AssistantProgress { .. }
            | ChatMessage::AssistantToolCall { .. }
            | ChatMessage::ToolResult { .. } => MessageFlags::recorded_context_display(),
            ChatMessage::Runtime { flags, .. } => *flags,
        }
    }

    pub fn should_record(&self) -> bool {
        self.flags().contains(MessageFlags::RECORD)
    }

    pub fn should_include_in_context(&self) -> bool {
        self.flags().contains(MessageFlags::CONTEXT)
    }

    pub fn should_display(&self) -> bool {
        self.flags().contains(MessageFlags::DISPLAY)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepDiagnostics {
    #[serde(rename = "stopReason")]
    pub stop_reason: Option<String>,
    #[serde(rename = "blockTypes")]
    pub block_types: Option<Vec<String>>,
    #[serde(rename = "ignoredBlockTypes")]
    pub ignored_block_types: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentStep {
    #[serde(rename = "assistant")]
    Assistant {
        content: String,
        kind: Option<String>,
        diagnostics: Option<StepDiagnostics>,
    },
    #[serde(rename = "tool_calls")]
    ToolCalls {
        calls: Vec<ToolCall>,
        content: Option<String>,
        #[serde(rename = "contentKind")]
        content_kind: Option<String>,
        diagnostics: Option<StepDiagnostics>,
    },
}

#[async_trait]
pub trait ModelAdapter: Send + Sync {
    /// 基于当前对话消息生成下一步代理动作。
    async fn next(&self, messages: &[ChatMessage]) -> anyhow::Result<AgentStep>;

    /// 对历史消息进行摘要总结。
    /// 默认使用基于规则的 fallback 实现。
    async fn summarize_conversation(&self, messages: &[ChatMessage]) -> Option<String> {
        let _ = messages;
        None
    }
}

/// Represents the current permissions summary for the session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionSummaryItem {
    Cwd(String),
    ExtraAllowDirs(Vec<String>),
    DangerousAllowDirs(Vec<String>),
}

impl Display for PermissionSummaryItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionSummaryItem::Cwd(cwd) => write!(f, "cwd: {}", cwd),
            PermissionSummaryItem::ExtraAllowDirs(dirs) => {
                if dirs.is_empty() {
                    write!(f, "extra allowed dirs: none")
                } else {
                    write!(f, "extra allowed dirs: {}", dirs.join(", "))
                }
            }
            PermissionSummaryItem::DangerousAllowDirs(cmds) => {
                if cmds.is_empty() {
                    write!(f, "dangerous allowlist: none")
                } else {
                    write!(f, "dangerous allowlist: {}", cmds.join(", "))
                }
            }
        }
    }
}

static MODEL_ADAPTER: OnceLock<Arc<dyn ModelAdapter>> = OnceLock::new();

pub fn set_model_adapter(adapter: Arc<dyn ModelAdapter>) -> anyhow::Result<()> {
    MODEL_ADAPTER
        .set(adapter)
        .map_err(|_| anyhow::anyhow!("Model adapter has already been set"))
}

pub fn get_model_adapter() -> &'static Arc<dyn ModelAdapter> {
    MODEL_ADAPTER.get().expect("Model adapter is not set")
}
