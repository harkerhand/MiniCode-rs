use minicode_types::ChatMessage;

/// 上下文利用率阈值：低于此值不触发微压缩
const MICROCOMPACT_UTILIZATION: f64 = 0.50;

/// 保留最近 N 条 tool_result 的完整内容
const KEEP_RECENT_TOOL_RESULTS: usize = 3;

/// 可清理的工具列表（只读工具）
const COMPACTABLE_TOOLS: &[&str] = &[
    "read_file",
    "list_files",
    "grep_files",
    "web_fetch",
    "web_search",
];

/// 清理标记
const CLEAR_MARKER: &str = "[Content cleared to save context space]";

/// 微压缩：清理旧的 tool_result 内容以节省 token
///
/// 这是一个轻量级的压缩策略，不调用模型，纯本地操作：
/// - 触发阈值：50% 上下文利用率
/// - 策略：将较旧的 tool_result 消息内容替换为清理标记
/// - 保留最近 N 条 tool_result 的完整内容
/// - 仅适用于只读工具（read_file, list_files 等）
pub fn microcompact(messages: Vec<ChatMessage>, context_utilization: f64) -> Vec<ChatMessage> {
    // 低于阈值不触发
    if context_utilization < MICROCOMPACT_UTILIZATION {
        return messages;
    }

    // 找到所有可清理的 tool_result 索引
    let tool_result_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, msg)| {
            if let ChatMessage::ToolResult { tool_name, .. } = msg {
                COMPACTABLE_TOOLS.contains(&tool_name.as_str())
            } else {
                false
            }
        })
        .map(|(i, _)| i)
        .collect();

    // 如果可清理的 tool_result 数量不足，不触发
    if tool_result_indices.len() <= KEEP_RECENT_TOOL_RESULTS {
        return messages;
    }

    // 计算需要清理的索引（保留最近 N 条）
    let clear_from = tool_result_indices.len() - KEEP_RECENT_TOOL_RESULTS;
    let indices_to_clear: std::collections::HashSet<usize> = tool_result_indices[..clear_from]
        .iter()
        .copied()
        .collect();

    // 替换内容为清理标记
    let mut changed = false;
    let result: Vec<ChatMessage> = messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            if indices_to_clear.contains(&i) {
                if let ChatMessage::ToolResult {
                    tool_use_id,
                    tool_name,
                    content,
                    ..
                } = msg
                {
                    // 只有内容不是清理标记时才替换
                    if content != CLEAR_MARKER {
                        changed = true;
                        ChatMessage::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            tool_name: tool_name.clone(),
                            content: CLEAR_MARKER.to_string(),
                            is_error: false,
                        }
                    } else {
                        msg.clone()
                    }
                } else {
                    msg.clone()
                }
            } else {
                msg.clone()
            }
        })
        .collect();

    if changed { result } else { messages }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool_result(tool_name: &str, content: &str) -> ChatMessage {
        ChatMessage::ToolResult {
            tool_use_id: "test-id".to_string(),
            tool_name: tool_name.to_string(),
            content: content.to_string(),
            is_error: false,
        }
    }

    #[test]
    fn test_below_threshold() {
        let messages = vec![
            make_tool_result("read_file", "content1"),
            make_tool_result("read_file", "content2"),
        ];
        let result = microcompact(messages.clone(), 0.3);
        assert_eq!(result.len(), 2);
        // 不应该被清理
        if let ChatMessage::ToolResult { content, .. } = &result[0] {
            assert_eq!(content, "content1");
        }
    }

    #[test]
    fn test_compact_old_results() {
        let messages = vec![
            make_tool_result("read_file", "old1"),
            make_tool_result("read_file", "old2"),
            make_tool_result("read_file", "old3"),
            make_tool_result("read_file", "recent1"),
            make_tool_result("read_file", "recent2"),
            make_tool_result("read_file", "recent3"),
        ];
        let result = microcompact(messages, 0.6);
        assert_eq!(result.len(), 6);

        // 前 3 条应该被清理
        if let ChatMessage::ToolResult { content, .. } = &result[0] {
            assert_eq!(content, CLEAR_MARKER);
        }
        if let ChatMessage::ToolResult { content, .. } = &result[1] {
            assert_eq!(content, CLEAR_MARKER);
        }
        if let ChatMessage::ToolResult { content, .. } = &result[2] {
            assert_eq!(content, CLEAR_MARKER);
        }

        // 后 3 条应该保留
        if let ChatMessage::ToolResult { content, .. } = &result[3] {
            assert_eq!(content, "recent1");
        }
        if let ChatMessage::ToolResult { content, .. } = &result[4] {
            assert_eq!(content, "recent2");
        }
        if let ChatMessage::ToolResult { content, .. } = &result[5] {
            assert_eq!(content, "recent3");
        }
    }

    #[test]
    fn test_skip_non_compactable_tools() {
        let messages = vec![
            make_tool_result("edit_file", "should not clear"),
            make_tool_result("read_file", "old1"),
            make_tool_result("read_file", "old2"),
            make_tool_result("read_file", "old3"),
            make_tool_result("read_file", "recent1"),
            make_tool_result("read_file", "recent2"),
            make_tool_result("read_file", "recent3"),
        ];
        let result = microcompact(messages, 0.6);

        // edit_file 不应该被清理
        if let ChatMessage::ToolResult { content, .. } = &result[0] {
            assert_eq!(content, "should not clear");
        }
    }

    #[test]
    fn test_already_cleared() {
        let messages = vec![
            make_tool_result("read_file", CLEAR_MARKER),
            make_tool_result("read_file", CLEAR_MARKER),
            make_tool_result("read_file", CLEAR_MARKER),
            make_tool_result("read_file", "recent1"),
            make_tool_result("read_file", "recent2"),
            make_tool_result("read_file", "recent3"),
        ];
        let result = microcompact(messages.clone(), 0.6);
        // 已经清理过的不应该改变
        assert_eq!(result.len(), 6);
    }
}
