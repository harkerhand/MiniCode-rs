use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{RuntimeConfig, load_runtime_config};
use crate::tool::ToolRegistry;
use crate::types::{AgentStep, ChatMessage, ModelAdapter, StepDiagnostics, ToolCall};

const DEFAULT_MAX_RETRIES: usize = 4;
const BASE_RETRY_DELAY_MS: u64 = 500;
const MAX_RETRY_DELAY_MS: u64 = 8_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicResponse {
    stop_reason: Option<String>,
    content: Option<Vec<Value>>,
}

pub struct AnthropicModelAdapter {
    client: reqwest::Client,
    tools: Arc<ToolRegistry>,
    cwd: std::path::PathBuf,
}

impl AnthropicModelAdapter {
    pub fn new(tools: Arc<ToolRegistry>, cwd: std::path::PathBuf) -> Self {
        Self {
            client: reqwest::Client::new(),
            tools,
            cwd,
        }
    }

    fn parse_assistant_text(content: &str) -> (String, Option<String>) {
        let trimmed = content.trim();
        if trimmed.starts_with("<final>") || trimmed.starts_with("[FINAL]") {
            return (
                trimmed
                    .trim_start_matches("<final>")
                    .trim_start_matches("[FINAL]")
                    .replace("</final>", "")
                    .trim()
                    .to_string(),
                Some("final".to_string()),
            );
        }
        if trimmed.starts_with("<progress>") || trimmed.starts_with("[PROGRESS]") {
            return (
                trimmed
                    .trim_start_matches("<progress>")
                    .trim_start_matches("[PROGRESS]")
                    .replace("</progress>", "")
                    .trim()
                    .to_string(),
                Some("progress".to_string()),
            );
        }
        (trimmed.to_string(), None)
    }

    fn should_retry(status: u16) -> bool {
        status == 429 || (500..600).contains(&status)
    }

    fn get_retry_limit() -> usize {
        std::env::var("MINI_CODE_MAX_RETRIES")
            .ok()
            .and_then(|x| x.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_RETRIES)
    }

    fn retry_delay_ms(attempt: usize, retry_after_ms: Option<u64>) -> u64 {
        if let Some(ms) = retry_after_ms {
            return ms;
        }
        let base = (BASE_RETRY_DELAY_MS * (2u64.saturating_pow((attempt.saturating_sub(1)) as u32))).min(MAX_RETRY_DELAY_MS);
        let mut rng = rand::rng();
        let jitter: f64 = rng.random_range(0.0..0.25);
        (base as f64 * (1.0 + jitter)) as u64
    }

    fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
        let raw = headers.get("retry-after")?.to_str().ok()?;
        if let Ok(sec) = raw.parse::<u64>() {
            return Some(sec * 1000);
        }
        None
    }

    fn to_anthropic_messages(messages: &[ChatMessage]) -> (String, Vec<AnthropicMessage>) {
        let mut system = vec![];
        let mut converted: Vec<AnthropicMessage> = vec![];

        let push = |arr: &mut Vec<AnthropicMessage>, role: &str, block: Value| {
            if let Some(last) = arr.last_mut()
                && last.role == role
            {
                last.content.push(block);
                return;
            }
            arr.push(AnthropicMessage {
                role: role.to_string(),
                content: vec![block],
            });
        };

        for msg in messages {
            match msg {
                ChatMessage::System { content } => system.push(content.clone()),
                ChatMessage::User { content } => {
                    push(&mut converted, "user", json!({"type":"text","text":content}));
                }
                ChatMessage::Assistant { content } => {
                    push(&mut converted, "assistant", json!({"type":"text","text":content}));
                }
                ChatMessage::AssistantProgress { content } => {
                    push(&mut converted, "assistant", json!({"type":"text","text":format!("<progress>\n{}\n</progress>", content)}));
                }
                ChatMessage::AssistantToolCall {
                    tool_use_id,
                    tool_name,
                    input,
                } => {
                    push(&mut converted, "assistant", json!({"type":"tool_use","id":tool_use_id,"name":tool_name,"input":input}));
                }
                ChatMessage::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    ..
                } => {
                    push(&mut converted, "user", json!({"type":"tool_result","tool_use_id":tool_use_id,"content":content,"is_error":is_error}));
                }
            }
        }

        (system.join("\n\n"), converted)
    }

    async fn get_runtime(&self) -> Result<RuntimeConfig> {
        load_runtime_config(&self.cwd)
    }
}

#[async_trait]
impl ModelAdapter for AnthropicModelAdapter {
    async fn next(&self, messages: &[ChatMessage]) -> Result<AgentStep> {
        let runtime = self.get_runtime().await?;
        let (system, anth_messages) = Self::to_anthropic_messages(messages);

        let tool_defs: Vec<Value> = self
            .tools
            .list()
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                })
            })
            .collect();

        let url = format!("{}/v1/messages", runtime.base_url.trim_end_matches('/'));
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("anthropic-version", reqwest::header::HeaderValue::from_static("2023-06-01"));

        if let Some(token) = runtime.auth_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        } else if let Some(api_key) = runtime.api_key {
            headers.insert("x-api-key", reqwest::header::HeaderValue::from_str(&api_key)?);
        }

        let body = json!({
            "model": runtime.model,
            "system": system,
            "messages": anth_messages,
            "tools": tool_defs,
            "max_tokens": runtime.max_output_tokens,
        });

        let retry_limit = Self::get_retry_limit();
        let mut last_status = 0;
        let mut last_err = String::new();

        for attempt in 0..=retry_limit {
            let resp = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await?;

            last_status = resp.status().as_u16();
            let retry_after = Self::parse_retry_after(resp.headers());
            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                last_err = text.clone();
                if Self::should_retry(last_status) && attempt < retry_limit {
                    tokio::time::sleep(Duration::from_millis(Self::retry_delay_ms(attempt + 1, retry_after))).await;
                    continue;
                }
                return Err(anyhow!("Model request failed: {} {}", last_status, text));
            }

            let data: AnthropicResponse = resp.json().await?;
            let mut tool_calls = vec![];
            let mut text_parts = vec![];
            let mut block_types = vec![];
            let mut ignored_block_types = vec![];

            for block in data.content.unwrap_or_default() {
                let t = block.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
                block_types.push(t.clone());
                if t == "text" {
                    if let Some(txt) = block.get("text").and_then(|x| x.as_str()) {
                        text_parts.push(txt.to_string());
                    }
                } else if t == "tool_use" {
                    let id = block.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                    let name = block.get("name").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    if !id.is_empty() && !name.is_empty() {
                        tool_calls.push(ToolCall { id, tool_name: name, input });
                    }
                } else {
                    ignored_block_types.push(t);
                }
            }

            let (content, kind) = Self::parse_assistant_text(&text_parts.join("\n"));
            let diagnostics = Some(StepDiagnostics {
                stop_reason: data.stop_reason,
                block_types: Some(block_types),
                ignored_block_types: Some(ignored_block_types),
            });

            if !tool_calls.is_empty() {
                return Ok(AgentStep::ToolCalls {
                    calls: tool_calls,
                    content: if content.is_empty() { None } else { Some(content) },
                    content_kind: kind,
                    diagnostics,
                });
            }

            return Ok(AgentStep::Assistant {
                content,
                kind,
                diagnostics,
            });
        }

        Err(anyhow!("Model request failed after retries: status={} err={}", last_status, last_err))
    }
}
