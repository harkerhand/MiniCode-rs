use minicode_types::ChatMessage;

const MESSAGE_OVERHEAD_TOKENS: usize = 4;

fn estimate_text_tokens(text: &str) -> usize {
    // Lightweight heuristic: roughly 1 token ~= 3 UTF-8 bytes.
    text.len().div_ceil(3)
}

/// Estimate token usage for the full conversation context sent to the model.
pub fn estimate_context_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|msg| msg.should_include_in_context())
        .map(|msg| {
            let content_tokens = match msg {
                ChatMessage::System { content }
                | ChatMessage::Minicode { content }
                | ChatMessage::User { content }
                | ChatMessage::Assistant { content }
                | ChatMessage::AssistantProgress { content } => estimate_text_tokens(content),
                ChatMessage::AssistantToolCall {
                    tool_use_id,
                    tool_name,
                    input,
                } => {
                    estimate_text_tokens(tool_use_id)
                        + estimate_text_tokens(tool_name)
                        + estimate_text_tokens(&input.to_string())
                }
                ChatMessage::ToolResult {
                    tool_use_id,
                    tool_name,
                    content,
                    ..
                } => {
                    estimate_text_tokens(tool_use_id)
                        + estimate_text_tokens(tool_name)
                        + estimate_text_tokens(content)
                }
                ChatMessage::Runtime { content, .. } | ChatMessage::ContextSummary { content } => {
                    estimate_text_tokens(content)
                }
            };
            MESSAGE_OVERHEAD_TOKENS + content_tokens
        })
        .sum()
}
