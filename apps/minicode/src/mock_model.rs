use anyhow::Result;
use async_trait::async_trait;

use crate::types::{AgentStep, ChatMessage, ModelAdapter};

pub struct MockModelAdapter;

#[async_trait]
impl ModelAdapter for MockModelAdapter {
    async fn next(&self, _messages: &[ChatMessage]) -> Result<AgentStep> {
        Ok(AgentStep::Assistant {
            content: "<final>Mock 模型响应：已收到请求。".to_string(),
            kind: Some("final".to_string()),
            diagnostics: None,
        })
    }
}
