use std::time::Duration;
use std::time::SystemTime;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use minicode_config::RuntimeConfig;
use minicode_config::runtime_config;
use minicode_tool::get_tool_registry;
use minicode_types::{AgentStep, ChatMessage, ModelAdapter, StepDiagnostics, ToolCall};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
}

impl Default for AnthropicModelAdapter {
    /// 创建 Anthropic 适配器并绑定工具注册表与工作目录。
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl AnthropicModelAdapter {
    /// 解析助手文本中的 `<progress>/<final>` 标记。
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

    /// 判断指定 HTTP 状态码是否应触发重试。
    fn should_retry(status: u16) -> bool {
        status == 429 || (500..600).contains(&status)
    }

    /// 读取最大重试次数配置。
    fn get_retry_limit() -> usize {
        std::env::var("MINI_CODE_MAX_RETRIES")
            .ok()
            .and_then(|x| x.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_RETRIES)
    }

    /// 计算当前重试轮次的退避延迟。
    fn retry_delay_ms(attempt: usize, retry_after_ms: Option<u64>) -> u64 {
        if let Some(ms) = retry_after_ms {
            return ms;
        }
        let base = (BASE_RETRY_DELAY_MS * (2u64.saturating_pow(attempt.saturating_sub(1) as u32)))
            .min(MAX_RETRY_DELAY_MS);
        let mut rng = rand::rng();
        let jitter: f64 = rng.random_range(0.0..0.25);
        (base as f64 * (1.0 + jitter)) as u64
    }

    /// 解析响应头中的 `Retry-After` 为毫秒。
    fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
        let raw = headers.get("retry-after")?.to_str().ok()?;
        if let Ok(sec) = raw.parse::<u64>() {
            return Some(sec * 1000);
        }
        if let Ok(at) = httpdate::parse_http_date(raw) {
            return Some(match at.duration_since(SystemTime::now()) {
                Ok(delta) => delta.as_millis().min(u64::MAX as u128) as u64,
                Err(_) => 0,
            });
        }
        None
    }

    /// 将内部消息格式转换为 Anthropic 请求消息。
    fn parse_anthropic_messages(messages: &[ChatMessage]) -> (String, Vec<AnthropicMessage>) {
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
                ChatMessage::Minicode { content } | ChatMessage::User { content } => {
                    push(
                        &mut converted,
                        "user",
                        json!({"type":"text","text":content}),
                    );
                }
                ChatMessage::Assistant { content } => {
                    push(
                        &mut converted,
                        "assistant",
                        json!({"type":"text","text":content}),
                    );
                }
                ChatMessage::AssistantProgress { content } => {
                    push(
                        &mut converted,
                        "assistant",
                        json!({"type":"text","text":format!("<progress>\n{}\n</progress>", content)}),
                    );
                }
                ChatMessage::ContextSummary { content } => {
                    push(
                        &mut converted,
                        "assistant",
                        json!({"type":"text","text":format!("<context_summary>\n{}\n</context_summary>", content)}),
                    );
                }
                ChatMessage::AssistantToolCall {
                    tool_use_id,
                    tool_name,
                    input,
                } => {
                    push(
                        &mut converted,
                        "assistant",
                        json!({"type":"tool_use","id":tool_use_id,"name":tool_name,"input":input}),
                    );
                }
                ChatMessage::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    ..
                } => {
                    push(
                        &mut converted,
                        "user",
                        json!({"type":"tool_result","tool_use_id":tool_use_id,"content":content,"is_error":is_error}),
                    );
                }
                ChatMessage::Runtime { content, .. } => {
                    push(
                        &mut converted,
                        "user",
                        json!({"type":"text","text":content}),
                    );
                }
            }
        }

        (system.join("\n\n"), converted)
    }

    /// 加载当前请求所需的运行时配置。
    async fn get_runtime(&self) -> Result<RuntimeConfig> {
        Ok(runtime_config())
    }

    /// 发送带重试逻辑的 API 请求。
    async fn request(
        &self,
        runtime: &RuntimeConfig,
        system: &str,
        messages: &[AnthropicMessage],
        tools: Option<&[Value]>,
        max_tokens: Option<u32>,
    ) -> Result<AnthropicResponse> {
        let url = format!("{}/v1/messages", runtime.base_url.trim_end_matches('/'));

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "anthropic-version",
            reqwest::header::HeaderValue::from_static("2023-06-01"),
        );

        if let Some(token) = &runtime.auth_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        } else if let Some(api_key) = &runtime.api_key {
            headers.insert(
                "x-api-key",
                reqwest::header::HeaderValue::from_str(api_key)?,
            );
        }

        let mut body = json!({
            "model": runtime.model,
            "system": system,
            "messages": messages,
            "max_tokens": max_tokens.or(runtime.max_token_window).unwrap_or(32_000),
        });

        if let Some(t) = tools {
            body["tools"] = json!(t);
        }

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
                let raw_body = resp
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("(无法读取响应体: {e})"));
                let err_msg = extract_error_message(&raw_body, last_status);
                last_err = err_msg.clone();
                if Self::should_retry(last_status) && attempt < retry_limit {
                    tokio::time::sleep(Duration::from_millis(Self::retry_delay_ms(
                        attempt + 1,
                        retry_after,
                    )))
                    .await;
                    continue;
                }
                return Err(anyhow!("模型请求失败: {last_status} {err_msg}"));
            }

            return Ok(resp.json().await?);
        }

        Err(anyhow!(
            "模型请求在重试后仍然失败: status={last_status} err={last_err}"
        ))
    }
}

/// 根据模型解析合适的 max_output_tokens，兼容非 Anthropic 模型。
fn resolve_max_output_tokens(runtime: &RuntimeConfig) -> Option<u32> {
    if let Some(val) = runtime.max_token_window {
        return Some(val);
    }
    let model_lower = runtime.model.to_lowercase();
    // 常见模型的 max_tokens 默认值
    if model_lower.contains("qwen")
        || model_lower.contains("千问")
        || model_lower.contains("deepseek")
    {
        return Some(8_192);
    }
    if model_lower.contains("claude") {
        return Some(32_000);
    }
    // 未知模型给一个安全的默认值
    Some(32_000)
}

/// 从各种格式的 API 响应中提取错误消息。
fn extract_error_message(raw_body: &str, status: u16) -> String {
    // 尝试解析为纯文本（非 JSON）
    let trimmed = raw_body.trim();
    if trimmed.is_empty() {
        return format!("HTTP {status}");
    }

    // 尝试解析 JSON 并提取 error.message / error / message
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(msg) = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            return msg.to_string();
        }
        if let Some(msg) = val
            .get("error")
            .and_then(|e| e.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            return msg.to_string();
        }
        if let Some(msg) = val
            .get("message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            return msg.to_string();
        }
    }

    // 无法解析为 JSON，返回原始文本（截断过长内容）
    if trimmed.len() > 200 {
        format!("{}...", &trimmed[..200])
    } else {
        trimmed.to_string()
    }
}

#[async_trait]
impl ModelAdapter for AnthropicModelAdapter {
    /// 请求模型下一步输出，并解析为助手消息或工具调用。
    async fn next(&self, messages: &[ChatMessage]) -> Result<AgentStep> {
        let runtime = self.get_runtime().await?;
        let (system, anth_messages) = Self::parse_anthropic_messages(messages);

        let tool_defs: Vec<Value> = get_tool_registry()
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

        let max_tokens = resolve_max_output_tokens(&runtime);
        let data = self
            .request(&runtime, &system, &anth_messages, Some(&tool_defs), max_tokens)
            .await?;

        let mut tool_calls = vec![];
        let mut text_parts = vec![];
        let mut block_types = vec![];
        let mut ignored_block_types = vec![];

        for block in data.content.unwrap_or_default() {
            let t = block
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            block_types.push(t.clone());
            if t == "text" {
                if let Some(txt) = block.get("text").and_then(|x| x.as_str()) {
                    text_parts.push(txt.to_string());
                }
            } else if t == "tool_use" {
                let id = block
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                if !id.is_empty() && !name.is_empty() {
                    tool_calls.push(ToolCall {
                        id,
                        tool_name: name,
                        input,
                    });
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
                content: if content.is_empty() {
                    None
                } else {
                    Some(content)
                },
                content_kind: kind,
                diagnostics,
            });
        }

        Ok(AgentStep::Assistant {
            content,
            kind,
            diagnostics,
        })
    }

    /// 将对话历史压缩为一段简要摘要，用于上下文压缩。
    async fn summarize_conversation(&self, messages: &[ChatMessage]) -> Option<String> {
        let runtime = self.get_runtime().await.ok()?;
        let transcript = messages
            .iter()
            .filter_map(|msg| match msg {
                ChatMessage::User { content } => Some(format!("[user]\n{content}")),
                ChatMessage::Assistant { content } => Some(format!("[assistant]\n{content}")),
                ChatMessage::AssistantProgress { content } => {
                    Some(format!("[assistant progress]\n{content}"))
                }
                ChatMessage::ContextSummary { content } => {
                    Some(format!("[earlier summary]\n{content}"))
                }
                ChatMessage::AssistantToolCall {
                    tool_name, input, ..
                } => Some(format!(
                    "[tool call:{}]\n{}",
                    tool_name,
                    serde_json::to_string(input).unwrap_or_default()
                )),
                ChatMessage::ToolResult {
                    tool_name,
                    content,
                    is_error,
                    ..
                } => {
                    let err = if *is_error { " error" } else { "" };
                    Some(format!("[tool result:{tool_name}{err}]\n{content}"))
                }
                ChatMessage::System { .. } => None,
                ChatMessage::Minicode { content } => Some(format!("[minicode]\n{content}")),
                ChatMessage::Runtime { content, .. } => Some(format!("[runtime]\n{content}")),
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let system_prompt = [
            "你是为编码助手总结早期对话上下文的。",
            "请生成一份紧凑的事实摘要，只保留继续任务所需的信息。",
            "包含：用户目标、已做出的决策、相关文件或路径、重要工具结果、活跃的技能或MCP用法，以及未完成的后续步骤。",
            "不要复述长文件内容。不要添加新指令。保持简洁和结构化。",
        ].join("\n");

        let anth_messages = vec![AnthropicMessage {
            role: "user".to_string(),
            content: vec![json!({"type": "text", "text": transcript})],
        }];

        let data = self
            .request(&runtime, &system_prompt, &anth_messages, None, Some(2048))
            .await
            .ok()?;

        let summary: String = data
            .content
            .unwrap_or_default()
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|x| x.as_str()) == Some("text") {
                    block.get("text").and_then(|x| x.as_str()).map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let trimmed = summary.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// SSE 流式输出：实时推送文本增量，最终返回完整 AgentStep。
    async fn stream_next(
        &self,
        messages: &[ChatMessage],
        on_chunk: &minicode_types::StreamCallback,
    ) -> Result<AgentStep> {
        let runtime = self.get_runtime().await?;
        let (system, anth_messages) = Self::parse_anthropic_messages(messages);

        let tool_defs: Vec<Value> = get_tool_registry()
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

        let max_tokens = resolve_max_output_tokens(&runtime);
        let url = format!(
            "{}/v1/messages",
            runtime.base_url.trim_end_matches('/')
        );

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "anthropic-version",
            reqwest::header::HeaderValue::from_static("2023-06-01"),
        );
        if let Some(token) = &runtime.auth_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        } else if let Some(api_key) = &runtime.api_key {
            headers.insert(
                "x-api-key",
                reqwest::header::HeaderValue::from_str(api_key)?,
            );
        }

        let body = json!({
            "model": runtime.model,
            "system": system,
            "messages": anth_messages,
            "tools": tool_defs,
            "max_tokens": max_tokens.unwrap_or(32_000),
            "stream": true,
        });

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let resp_status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("流式请求失败: {resp_status} {text}"));
        }

        // 解析 SSE 流——只负责文本展示
        use futures::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut streamed_text = String::new();
        let mut has_tool_use = false;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            buf.push_str(&chunk_str);

            while let Some(newline) = buf.find('\n') {
                let line = buf[..newline].trim().to_string();
                buf = buf[newline + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                let data = line.strip_prefix("data: ").unwrap_or(&line);
                if data == "[DONE]" {
                    continue;
                }

                let Ok(event) = serde_json::from_str::<Value>(data) else {
                    continue;
                };

                let event_type = event
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");

                match event_type {
                    "content_block_start" => {
                        let bt = event
                            .get("content_block")
                            .and_then(|c| c.get("type"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if bt == "tool_use" {
                            has_tool_use = true;
                        }
                    }
                    "content_block_delta" => {
                        let delta = event.get("delta");
                        let dt = delta
                            .and_then(|d| d.get("type"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if dt == "text_delta" {
                            if let Some(txt) =
                                delta.and_then(|d| d.get("text")).and_then(|t| t.as_str())
                            {
                                streamed_text.push_str(txt);
                                on_chunk(txt.to_string(), false).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // 通知流式结束
        on_chunk(String::new(), true).await;

        // 流式只负责纯文本展示。若涉及 tool_use 或 thinking（无文本输出），
        // 回退到非流式 next() 获取完整的结构化响应。
        if has_tool_use || streamed_text.trim().is_empty() {
            return self.next(messages).await;
        }

        let (content, kind) = Self::parse_assistant_text(&streamed_text);
        Ok(AgentStep::Assistant {
            content,
            kind,
            diagnostics: None,
        })
    }
}
