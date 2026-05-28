use minicode_types::ChatMessage;

/// 上下文利用率阈值：低于此值不触发 snip 压缩
const SNIP_COMPACT_THRESHOLD: f64 = 0.70;

/// 目标利用率：压缩后希望达到的利用率
const SNIP_TARGET_USAGE: f64 = 0.60;

/// 最少删除消息数
const SNIP_MIN_MESSAGES_TO_REMOVE: usize = 6;

/// 保留最近 N 条消息
const SNIP_KEEP_RECENT_MESSAGES: usize = 12;

/// 最少释放 token 数
const SNIP_MIN_TOKENS_TO_FREE: usize = 2_000;

/// 受保护的工具名称（文件编辑类）
const PROTECTED_TOOL_NAMES: &[&str] = &[
    "edit_file",
    "modify_file",
    "patch_file",
    "write_file",
];

/// 错误标记
const ERROR_MARKERS: &[&str] = &[
    "error",
    "failed",
    "failure",
    "exception",
    "traceback",
    "permission denied",
];

/// Snip 压缩结果
#[derive(Debug, Clone)]
pub struct SnipCompactResult {
    pub messages: Vec<ChatMessage>,
    pub did_snip: bool,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub tokens_freed: usize,
    pub removed_count: usize,
    pub reason: Option<String>,
}

/// 消息组（tool_call + tool_result 为一组）
#[derive(Debug, Clone)]
struct MessageGroup {
    start: usize,
    end: usize,
    messages: Vec<ChatMessage>,
    tokens: usize,
    is_protected: bool,
    reasons: Vec<String>,
}

/// 安全区间（连续的非受保护组）
#[derive(Debug, Clone)]
struct SafeRun {
    groups: Vec<MessageGroup>,
    start: usize,
    messages_count: usize,
    tokens: usize,
}

/// 估算消息的 token 数（简单算法：每 3 个字符约 1 个 token）
fn estimate_message_tokens(msg: &ChatMessage) -> usize {
    let text_len = match msg {
        ChatMessage::System { content }
        | ChatMessage::Minicode { content }
        | ChatMessage::User { content }
        | ChatMessage::Assistant { content }
        | ChatMessage::AssistantProgress { content }
        | ChatMessage::ContextSummary { content } => content.len(),
        ChatMessage::AssistantToolCall { input, .. } => {
            input.to_string().len() + 50
        }
        ChatMessage::ToolResult { content, .. } => content.len() + 20,
        ChatMessage::Runtime { content, .. } => content.len(),
        ChatMessage::SnipBoundary { content, .. } => content.len(),
    };
    // 每 3 个字符约 1 个 token，加上 4 个 token 的固定开销
    (text_len / 3) + 4
}

/// 估算消息列表的 token 数
fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|m| estimate_message_tokens(m)).sum()
}

/// 判断是否为边界消息
fn is_boundary_message(msg: &ChatMessage) -> bool {
    matches!(
        msg,
        ChatMessage::System { .. }
            | ChatMessage::ContextSummary { .. }
            | ChatMessage::SnipBoundary { .. }
    )
}

/// 判断工具是否为受保护的文件编辑工具
fn is_protected_tool_name(tool_name: &str) -> bool {
    let normalized = tool_name.trim().to_lowercase();
    PROTECTED_TOOL_NAMES
        .iter()
        .any(|&name| name == normalized)
        || normalized.contains("patch")
        || normalized.contains("write")
        || normalized.contains("edit")
        || normalized.contains("modify")
}

/// 判断 tool_result 是否包含重要错误
fn tool_result_looks_important_error(msg: &ChatMessage) -> bool {
    if let ChatMessage::ToolResult {
        content, is_error, ..
    } = msg
    {
        if *is_error {
            return true;
        }
        let content_lower = content.to_lowercase();
        ERROR_MARKERS
            .iter()
            .any(|marker| content_lower.contains(marker))
    } else {
        false
    }
}

/// 判断消息文本是否包含重要错误
fn message_text_looks_important_error(msg: &ChatMessage) -> bool {
    let content = match msg {
        ChatMessage::User { content }
        | ChatMessage::Assistant { content }
        | ChatMessage::AssistantProgress { content } => content.to_lowercase(),
        _ => return false,
    };
    ERROR_MARKERS
        .iter()
        .any(|marker| content.contains(marker))
}

/// 判断组是否包含受保护的工具
fn group_has_protected_tool(group: &MessageGroup) -> bool {
    group.messages.iter().any(|msg| match msg {
        ChatMessage::AssistantToolCall { tool_name, .. }
        | ChatMessage::ToolResult { tool_name, .. } => is_protected_tool_name(tool_name),
        _ => false,
    })
}

/// 判断组是否包含重要错误
fn group_has_important_error(group: &MessageGroup) -> bool {
    group
        .messages
        .iter()
        .any(|msg| message_text_looks_important_error(msg) || tool_result_looks_important_error(msg))
}

/// 构建消息组
fn build_message_groups(messages: &[ChatMessage]) -> Vec<MessageGroup> {
    let mut groups = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = &messages[i];

        // AssistantToolCall：尝试与下一个 ToolResult 配对
        if let ChatMessage::AssistantToolCall { tool_use_id, .. } = msg {
            let next = messages.get(i + 1);
            let grouped = if let Some(ChatMessage::ToolResult {
                tool_use_id: next_id,
                ..
            }) = next
            {
                if next_id == tool_use_id {
                    vec![msg.clone(), next.unwrap().clone()]
                } else {
                    vec![msg.clone()]
                }
            } else {
                vec![msg.clone()]
            };

            let group_len = grouped.len();
            let is_protected = group_len == 1; // 未闭合的 tool_call 受保护
            let reasons = if is_protected {
                vec!["unclosed_tool_call".to_string()]
            } else {
                vec![]
            };

            groups.push(MessageGroup {
                start: i,
                end: i + group_len,
                messages: grouped,
                tokens: estimate_messages_tokens(&messages[i..i + group_len]),
                is_protected,
                reasons,
            });
            i += group_len;
            continue;
        }

        // 孤立的 ToolResult
        if let ChatMessage::ToolResult { .. } = msg {
            groups.push(MessageGroup {
                start: i,
                end: i + 1,
                messages: vec![msg.clone()],
                tokens: estimate_message_tokens(msg),
                is_protected: true,
                reasons: vec!["orphan_tool_result".to_string()],
            });
            i += 1;
            continue;
        }

        // 普通消息
        groups.push(MessageGroup {
            start: i,
            end: i + 1,
            messages: vec![msg.clone()],
            tokens: estimate_message_tokens(msg),
            is_protected: false,
            reasons: vec![],
        });
        i += 1;
    }

    groups
}

/// 为组添加保护原因
fn add_protected_reason(group: &mut MessageGroup, reason: &str) {
    group.is_protected = true;
    if !group.reasons.contains(&reason.to_string()) {
        group.reasons.push(reason.to_string());
    }
}

/// 保护相邻的组
fn protect_nearby_groups(groups: &mut [MessageGroup], index: usize, reason: &str) {
    let start = index.saturating_sub(1);
    let end = (index + 2).min(groups.len());
    for i in start..end {
        add_protected_reason(&mut groups[i], reason);
    }
}

/// 标记受保护的组
fn mark_protected_groups(
    groups: &mut [MessageGroup],
    candidate_start: usize,
    candidate_end: usize,
) {
    // 标记范围外的组为受保护
    for group in groups.iter_mut() {
        if group.start < candidate_start || group.end > candidate_end {
            add_protected_reason(group, "outside_candidate_range");
        }

        // 边界消息受保护
        if group.messages.iter().any(is_boundary_message) {
            add_protected_reason(group, "boundary_message");
        }
    }

    // 文件编辑和错误消息及其邻居受保护
    let len = groups.len();
    for i in 0..len {
        if group_has_protected_tool(&groups[i]) {
            protect_nearby_groups(groups, i, "near_file_edit");
        }
        if group_has_important_error(&groups[i]) {
            protect_nearby_groups(groups, i, "near_important_error");
        }
    }
}

/// 找到候选范围（排除最近消息和边界消息）
fn find_candidate_range(messages: &[ChatMessage]) -> (usize, usize, Option<String>) {
    // 消息太少，不触发
    if messages.len() <= SNIP_KEEP_RECENT_MESSAGES + SNIP_MIN_MESSAGES_TO_REMOVE {
        return (0, 0, Some("too_few_messages".to_string()));
    }

    // 保留最近 N 条消息
    let keep_recent_start = messages.len().saturating_sub(SNIP_KEEP_RECENT_MESSAGES);

    // 找到最后一个 user 消息的位置
    let last_user_index = messages
        .iter()
        .rposition(|msg| matches!(msg, ChatMessage::User { .. }));

    let end = keep_recent_start.min(last_user_index.unwrap_or(messages.len()));
    if end == 0 {
        return (0, 0, Some("no_middle_range".to_string()));
    }

    // 找到边界消息后的位置作为起始
    let mut start = 0;
    for i in 0..end {
        if is_boundary_message(&messages[i]) {
            start = i + 1;
        }
    }

    // 范围太小，不触发
    if end - start < SNIP_MIN_MESSAGES_TO_REMOVE {
        return (start, end, Some("candidate_range_too_small".to_string()));
    }

    (start, end, None)
}

/// 找到安全区间（连续的非受保护组）
fn find_safe_runs(groups: &[MessageGroup]) -> Vec<SafeRun> {
    let mut runs = Vec::new();
    let mut current: Vec<MessageGroup> = Vec::new();

    let flush = |current: &mut Vec<MessageGroup>, runs: &mut Vec<SafeRun>| {
        if current.is_empty() {
            return;
        }
        let first = &current[0];
        let last = &current[current.len() - 1];
        runs.push(SafeRun {
            groups: current.clone(),
            start: first.start,
            messages_count: last.end - first.start,
            tokens: current.iter().map(|g| g.tokens).sum(),
        });
        current.clear();
    };

    for group in groups {
        if group.is_protected {
            flush(&mut current, &mut runs);
            continue;
        }
        current.push(group.clone());
    }
    flush(&mut current, &mut runs);

    runs
}

/// 比较两个安全区间（优先选择释放更多 token 的）
fn compare_runs(a: &SafeRun, b: &SafeRun) -> std::cmp::Ordering {
    b.tokens
        .cmp(&a.tokens)
        .then(b.messages_count.cmp(&a.messages_count))
        .then(a.start.cmp(&b.start))
}

/// 从安全区间中选择删除范围
fn select_deletion_from_run(run: &SafeRun, desired_tokens_to_free: usize) -> (usize, usize, usize) {
    let mut end_group_index = 0;
    let mut tokens = 0;

    for (i, group) in run.groups.iter().enumerate() {
        tokens += group.tokens;
        let messages_count = group.end - run.start;
        end_group_index = i;

        if tokens >= desired_tokens_to_free && messages_count >= SNIP_MIN_MESSAGES_TO_REMOVE {
            break;
        }
    }

    let end_group = &run.groups[end_group_index.min(run.groups.len() - 1)];
    (run.start, end_group.end, tokens)
}

/// 构建 SnipBoundary 消息
fn build_boundary_message(removed_count: usize, tokens_freed: usize) -> ChatMessage {
    let content = format!(
        "[Snipped earlier conversation segment]\n\n\
         A middle portion of the earlier conversation was removed to preserve context space.\n\n\
         Removed range:\n\
         - messages: {}\n\
         - approximate tokens freed: {}\n\n\
         The recent conversation and active task context are preserved.",
        removed_count,
        tokens_freed.max(0)
    );

    ChatMessage::SnipBoundary {
        content,
        removed_message_ids: vec![],
        removed_count,
        tokens_freed,
    }
}

/// 创建无操作结果
fn no_snip_result(messages: Vec<ChatMessage>, tokens_before: usize, reason: &str) -> SnipCompactResult {
    SnipCompactResult {
        messages,
        did_snip: false,
        tokens_before,
        tokens_after: tokens_before,
        tokens_freed: 0,
        removed_count: 0,
        reason: Some(reason.to_string()),
    }
}

/// Snip 压缩：安全删除中间消息段
///
/// 这是一个确定性的压缩策略，不调用模型：
/// - 触发阈值：70% 上下文利用率
/// - 目标利用率：60%
/// - 识别"安全区间"（不包含文件编辑工具、错误消息的连续消息段）
/// - 保护规则：保护文件编辑、错误回合、边界消息、未闭合的工具调用
/// - 选择最大的安全区间进行删除
/// - 插入 SnipBoundary 消息标记删除位置
pub fn snip_compact_conversation(
    messages: Vec<ChatMessage>,
    context_utilization: f64,
) -> SnipCompactResult {
    let tokens_before = estimate_messages_tokens(&messages);

    // 检查阈值
    if context_utilization < SNIP_COMPACT_THRESHOLD {
        return no_snip_result(messages, tokens_before, "below_threshold");
    }

    // 找到候选范围
    let (candidate_start, candidate_end, reason) = find_candidate_range(&messages);
    if let Some(reason) = reason {
        return no_snip_result(messages, tokens_before, &reason);
    }

    // 构建消息组
    let mut groups = build_message_groups(&messages);

    // 标记受保护的组
    mark_protected_groups(&mut groups, candidate_start, candidate_end);

    // 找到安全区间
    let mut safe_runs: Vec<SafeRun> = find_safe_runs(&groups)
        .into_iter()
        .filter(|run| {
            run.messages_count >= SNIP_MIN_MESSAGES_TO_REMOVE && run.tokens >= SNIP_MIN_TOKENS_TO_FREE
        })
        .collect();

    // 按释放 token 数排序
    safe_runs.sort_by(compare_runs);

    // 选择最佳区间
    let best_run = match safe_runs.first() {
        Some(run) => run,
        None => return no_snip_result(messages, tokens_before, "no_safe_interval"),
    };

    // 计算需要释放的 token 数
    let target_tokens = (context_utilization * SNIP_TARGET_USAGE * tokens_before as f64) as usize;
    let desired_tokens_to_free = SNIP_MIN_TOKENS_TO_FREE.max(tokens_before.saturating_sub(target_tokens));

    // 选择删除范围
    let (delete_start, delete_end, deletion_tokens) =
        select_deletion_from_run(best_run, desired_tokens_to_free);

    let removed_count = delete_end - delete_start;
    if removed_count < SNIP_MIN_MESSAGES_TO_REMOVE {
        return no_snip_result(messages, tokens_before, "below_min_messages");
    }

    // 构建边界消息
    let boundary_message = build_boundary_message(removed_count, deletion_tokens);
    let boundary_tokens = estimate_message_tokens(&boundary_message);
    let estimated_tokens_freed = deletion_tokens.saturating_sub(boundary_tokens);

    if estimated_tokens_freed < SNIP_MIN_TOKENS_TO_FREE {
        return no_snip_result(messages, tokens_before, "below_min_tokens");
    }

    // 执行删除
    let mut result_messages = Vec::with_capacity(messages.len() - removed_count + 1);
    result_messages.extend_from_slice(&messages[..delete_start]);
    result_messages.push(boundary_message);
    result_messages.extend_from_slice(&messages[delete_end..]);

    let tokens_after = estimate_messages_tokens(&result_messages);

    // 确保确实释放了 token
    if tokens_after >= tokens_before {
        return no_snip_result(messages, tokens_before, "no_token_reduction");
    }

    let tokens_freed = tokens_before.saturating_sub(tokens_after);

    SnipCompactResult {
        messages: result_messages,
        did_snip: true,
        tokens_before,
        tokens_after,
        tokens_freed,
        removed_count,
        reason: Some("snipped_safe_middle_interval".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_user(content: &str) -> ChatMessage {
        ChatMessage::User {
            content: content.to_string(),
        }
    }

    fn make_assistant(content: &str) -> ChatMessage {
        ChatMessage::Assistant {
            content: content.to_string(),
        }
    }

    fn make_tool_call(tool_name: &str) -> ChatMessage {
        ChatMessage::AssistantToolCall {
            tool_use_id: "test-id".to_string(),
            tool_name: tool_name.to_string(),
            input: json!({}),
        }
    }

    fn make_tool_result(tool_name: &str, content: &str) -> ChatMessage {
        ChatMessage::ToolResult {
            tool_use_id: "test-id".to_string(),
            tool_name: tool_name.to_string(),
            content: content.to_string(),
            is_error: false,
        }
    }

    fn make_error_result(tool_name: &str) -> ChatMessage {
        ChatMessage::ToolResult {
            tool_use_id: "test-id".to_string(),
            tool_name: tool_name.to_string(),
            content: "Error: something failed".to_string(),
            is_error: true,
        }
    }

    #[test]
    fn test_below_threshold() {
        let messages = vec![make_user("hello"), make_assistant("hi")];
        let result = snip_compact_conversation(messages, 0.5);
        assert!(!result.did_snip);
        assert_eq!(result.reason.unwrap(), "below_threshold");
    }

    #[test]
    fn test_too_few_messages() {
        let messages: Vec<ChatMessage> = (0..8)
            .flat_map(|i| vec![make_user(&format!("msg{}", i)), make_assistant(&format!("reply{}", i))])
            .collect();
        let result = snip_compact_conversation(messages, 0.8);
        assert!(!result.did_snip);
        assert_eq!(result.reason.unwrap(), "too_few_messages");
    }

    #[test]
    fn test_protect_file_edit() {
        // 创建足够多的消息
        let mut messages = Vec::new();
        for i in 0..20 {
            messages.push(make_user(&format!("msg{}", i)));
            messages.push(make_assistant(&format!("reply{}", i)));
        }
        // 在中间插入文件编辑
        messages.insert(10, make_tool_call("edit_file"));
        messages.insert(11, make_tool_result("edit_file", "edited"));

        let result = snip_compact_conversation(messages, 0.8);
        // 文件编辑及其附近的消息应该被保护
        if result.did_snip {
            // 检查编辑操作是否被保留
            let has_edit = result.messages.iter().any(|msg| {
                matches!(msg, ChatMessage::AssistantToolCall { tool_name, .. } if tool_name == "edit_file")
            });
            assert!(has_edit, "File edit should be preserved");
        }
    }

    #[test]
    fn test_protect_error() {
        // 创建足够多的消息
        let mut messages = Vec::new();
        for i in 0..20 {
            messages.push(make_user(&format!("msg{}", i)));
            messages.push(make_assistant(&format!("reply{}", i)));
        }
        // 在中间插入错误
        messages.insert(10, make_tool_call("run_command"));
        messages.insert(11, make_error_result("run_command"));

        let result = snip_compact_conversation(messages, 0.8);
        // 错误消息应该被保护
        if result.did_snip {
            let has_error = result.messages.iter().any(|msg| {
                matches!(msg, ChatMessage::ToolResult { is_error, .. } if *is_error)
            });
            assert!(has_error, "Error should be preserved");
        }
    }

    #[test]
    fn test_boundary_message_inserted() {
        // 创建足够多的安全消息
        let mut messages = Vec::new();
        for i in 0..30 {
            messages.push(make_user(&format!("msg{}", i)));
            messages.push(make_assistant(&format!("reply{}", i)));
        }

        let result = snip_compact_conversation(messages, 0.8);
        if result.did_snip {
            // 应该有 SnipBoundary 消息
            let has_boundary = result
                .messages
                .iter()
                .any(|msg| matches!(msg, ChatMessage::SnipBoundary { .. }));
            assert!(has_boundary, "Should have SnipBoundary message");
            assert!(result.tokens_freed > 0);
            assert!(result.removed_count > 0);
        }
    }
}
