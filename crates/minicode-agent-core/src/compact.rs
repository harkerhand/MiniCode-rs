use minicode_types::{ChatMessage, ModelAdapter};

/// 自动压缩的默认 token 阈值。
const DEFAULT_AUTO_COMPACT_THRESHOLD_TOKENS: usize = 128_000;
/// 压缩时保留的最近消息数量。
const DEFAULT_AUTO_COMPACT_PRESERVE_MESSAGES: usize = 12;
/// 摘要最大字符数。
const MAX_SUMMARY_CHARS: usize = 8_000;
/// 工具结果预览最大字符数。
const MAX_TOOL_RESULT_PREVIEW_CHARS: usize = 1_200;
/// 普通消息预览最大字符数。
const MAX_MESSAGE_PREVIEW_CHARS: usize = 2_000;

/// 判断消息是否为续写提示（不应纳入压缩）。
fn is_continuation_prompt(msg: &ChatMessage) -> bool {
    let ChatMessage::User { content } = msg else {
        return false;
    };
    content.starts_with("继续")
        || content.starts_with("Continue immediately")
        || content.starts_with("Continue from")
        || content.starts_with("Resume from")
        || content.starts_with("Your last response was empty")
        || content.starts_with("Your previous response hit max_tokens")
}

/// 截取文本预览片段。
fn preview_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= max_chars {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..max_chars])
    }
}

/// 将消息序列化为摘要行。
fn serialize_message_for_summary(msg: &ChatMessage) -> Option<String> {
    if is_continuation_prompt(msg) {
        return None;
    }

    match msg {
        ChatMessage::System { .. } => None,
        ChatMessage::User { content } => Some(format!(
            "[user]\n{}",
            preview_text(content, MAX_MESSAGE_PREVIEW_CHARS)
        )),
        ChatMessage::Assistant { content } => Some(format!(
            "[assistant]\n{}",
            preview_text(content, MAX_MESSAGE_PREVIEW_CHARS)
        )),
        ChatMessage::AssistantProgress { content } => Some(format!(
            "[assistant progress]\n{}",
            preview_text(content, MAX_MESSAGE_PREVIEW_CHARS)
        )),
        ChatMessage::ContextSummary { content } => Some(format!(
            "[earlier summary]\n{}",
            preview_text(content, MAX_MESSAGE_PREVIEW_CHARS)
        )),
        ChatMessage::AssistantToolCall {
            tool_name, input, ..
        } => Some(format!(
            "[tool call:{}]\n{}",
            tool_name,
            preview_text(&input.to_string(), MAX_MESSAGE_PREVIEW_CHARS)
        )),
        ChatMessage::ToolResult {
            tool_name,
            content,
            is_error,
            ..
        } => {
            let err_tag = if *is_error { " error" } else { "" };
            Some(format!(
                "[tool result:{tool_name}{err_tag}]\n{}",
                preview_text(content, MAX_TOOL_RESULT_PREVIEW_CHARS)
            ))
        }
        ChatMessage::Minicode { content } => Some(format!(
            "[minicode]\n{}",
            preview_text(content, MAX_MESSAGE_PREVIEW_CHARS)
        )),
        ChatMessage::Runtime { content, .. } => Some(format!(
            "[runtime]\n{}",
            preview_text(content, MAX_MESSAGE_PREVIEW_CHARS)
        )),
    }
}

/// 裁剪摘要至最大长度。
fn cap_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.len() <= MAX_SUMMARY_CHARS {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..MAX_SUMMARY_CHARS])
    }
}

/// 基于规则的降级摘要生成（不调用模型）。
fn fallback_summary(messages: &[ChatMessage]) -> String {
    let lines: Vec<String> = messages
        .iter()
        .filter_map(serialize_message_for_summary)
        .collect();

    let preview = preview_text(&lines.join("\n\n"), MAX_SUMMARY_CHARS);
    format!(
        "对话历史摘要:\n{}",
        if preview.is_empty() {
            "无重要的前期上下文。".to_string()
        } else {
            preview
        }
    )
}

/// 在对话上下文超出阈值时自动压缩。
///
/// 返回压缩后的消息列表，调用方的 `on_compact` 回调可用于 UI 提示。
pub async fn maybe_auto_compact_conversation(
    model: &dyn ModelAdapter,
    messages: Vec<ChatMessage>,
    threshold_tokens: Option<usize>,
    preserve_messages: Option<usize>,
    on_compact: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Vec<ChatMessage> {
    let threshold = threshold_tokens.unwrap_or(DEFAULT_AUTO_COMPACT_THRESHOLD_TOKENS);
    let preserve = preserve_messages.unwrap_or(DEFAULT_AUTO_COMPACT_PRESERVE_MESSAGES);

    let system_msgs: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| matches!(m, ChatMessage::System { .. }))
        .collect();
    let non_system: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| !matches!(m, ChatMessage::System { .. }))
        .collect();

    let estimated: usize = messages.iter().map(estimate_message_tokens).sum();

    if estimated < threshold || non_system.len() <= preserve + 1 {
        return messages;
    }

    let split_at = non_system.len().saturating_sub(preserve);
    let head: Vec<&ChatMessage> = non_system[..split_at].to_vec();
    let tail: Vec<&ChatMessage> = non_system[split_at..].to_vec();

    let head_filtered: Vec<&ChatMessage> = head
        .into_iter()
        .filter(|m| !is_continuation_prompt(m))
        .collect();

    if head_filtered.is_empty() {
        return messages;
    }

    let head_cloned: Vec<ChatMessage> = head_filtered.into_iter().cloned().collect();

    let summary = if let Some(s) = model.summarize_conversation(&head_cloned).await {
        cap_summary(&s)
    } else {
        fallback_summary(&head_cloned)
    };

    if let Some(cb) = on_compact {
        cb(&summary);
    }

    let mut result: Vec<ChatMessage> = system_msgs.into_iter().cloned().collect();
    result.push(ChatMessage::ContextSummary { content: summary });
    result.extend(tail.into_iter().cloned());
    result
}

/// 估算单条消息的 token 数（用于快速判断是否触及压缩阈值）。
fn estimate_message_tokens(msg: &ChatMessage) -> usize {
    const OVERHEAD: usize = 4;
    let content_tokens = match msg {
        ChatMessage::System { content }
        | ChatMessage::Minicode { content }
        | ChatMessage::User { content }
        | ChatMessage::Assistant { content }
        | ChatMessage::AssistantProgress { content }
        | ChatMessage::ContextSummary { content } => content.len().div_ceil(3),
        ChatMessage::AssistantToolCall {
            tool_use_id,
            tool_name,
            input,
        } => {
            tool_use_id.len().div_ceil(3)
                + tool_name.len().div_ceil(3)
                + input.to_string().len().div_ceil(3)
        }
        ChatMessage::ToolResult {
            tool_use_id,
            tool_name,
            content,
            ..
        } => {
            tool_use_id.len().div_ceil(3) + tool_name.len().div_ceil(3) + content.len().div_ceil(3)
        }
        ChatMessage::Runtime { content, .. } => content.len().div_ceil(3),
    };
    OVERHEAD + content_tokens
}
