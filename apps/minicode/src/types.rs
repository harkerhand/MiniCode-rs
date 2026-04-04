use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum ChatMessage {
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant { content: String },
    #[serde(rename = "assistant_progress")]
    AssistantProgress { content: String },
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
    async fn next(&self, messages: &[ChatMessage]) -> anyhow::Result<AgentStep>;
}
