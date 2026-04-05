use anyhow::Result;
use async_trait::async_trait;
use minicode_types::{AgentStep, ModelAdapter};

use crate::MockModelAdapter;
use crate::commands::{default_response, parse_user_command, render_tool_result_response};
use crate::helpers::{extract_latest_assistant_call, last_tool_message, last_user_message};

#[async_trait]
impl ModelAdapter for MockModelAdapter {
    /// 根据简单规则返回工具调用或最终回复，用于本地测试。
    async fn next(&self, messages: &[minicode_types::ChatMessage]) -> Result<AgentStep> {
        if let Some((_, _, content)) = last_tool_message(messages)
            && let Some(tool_name) = extract_latest_assistant_call(messages)
        {
            return Ok(render_tool_result_response(&tool_name, &content));
        }

        let user_text = last_user_message(messages).trim().to_string();
        if let Some(step) = parse_user_command(&user_text) {
            return Ok(step);
        }

        Ok(default_response())
    }
}
