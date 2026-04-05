use minicode_types::ChatMessage;

/// 取最近一条用户消息文本。
pub(crate) fn last_user_message(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|m| match m {
            ChatMessage::User { content } => Some(content.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// 取最近一条工具结果消息。
pub(crate) fn last_tool_message(messages: &[ChatMessage]) -> Option<(String, String, String)> {
    messages.iter().rev().find_map(|m| match m {
        ChatMessage::ToolResult {
            tool_use_id,
            tool_name,
            content,
            ..
        } => Some((tool_use_id.clone(), tool_name.clone(), content.clone())),
        _ => None,
    })
}

/// 取最近一次助手发起的工具调用名称。
pub(crate) fn extract_latest_assistant_call(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| match m {
        ChatMessage::AssistantToolCall { tool_name, .. } => Some(tool_name.clone()),
        _ => None,
    })
}
