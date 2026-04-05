mod adapter;
mod commands;
mod helpers;

pub struct MockModelAdapter;

#[cfg(test)]
mod tests {
    use minicode_types::{AgentStep, ChatMessage, ModelAdapter};

    use crate::MockModelAdapter;

    #[tokio::test]
    /// 验证 `/tools` 返回工具清单文本。
    async fn test_mock_model_tools_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/tools".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::Assistant { content, .. } => {
                assert!(content.contains("list_files"));
                assert!(content.contains("read_file"));
            }
            _ => panic!("Expected Assistant response"),
        }
    }

    #[tokio::test]
    /// 验证 `/ls` 会生成 `list_files` 工具调用。
    async fn test_mock_model_ls_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/ls src".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "list_files");
                assert_eq!(
                    calls[0].input.get("path").and_then(|v| v.as_str()),
                    Some("src")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }

    #[tokio::test]
    /// 验证 `/grep` 会生成 `grep_files` 工具调用。
    async fn test_mock_model_grep_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/grep fn main::src".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "grep_files");
                assert_eq!(
                    calls[0].input.get("pattern").and_then(|v| v.as_str()),
                    Some("fn main")
                );
                assert_eq!(
                    calls[0].input.get("path").and_then(|v| v.as_str()),
                    Some("src")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }

    #[tokio::test]
    /// 验证 `/read` 会生成 `read_file` 工具调用。
    async fn test_mock_model_read_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/read README.md".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "read_file");
                assert_eq!(
                    calls[0].input.get("path").and_then(|v| v.as_str()),
                    Some("README.md")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }

    #[tokio::test]
    /// 验证 `/write` 会生成 `write_file` 工具调用。
    async fn test_mock_model_write_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/write notes.txt::hello world".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "write_file");
                assert_eq!(
                    calls[0].input.get("path").and_then(|v| v.as_str()),
                    Some("notes.txt")
                );
                assert_eq!(
                    calls[0].input.get("content").and_then(|v| v.as_str()),
                    Some("hello world")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }

    #[tokio::test]
    /// 验证 `/edit` 会生成 `edit_file` 工具调用。
    async fn test_mock_model_edit_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/edit file.txt::old::new".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "edit_file");
                assert_eq!(
                    calls[0].input.get("path").and_then(|v| v.as_str()),
                    Some("file.txt")
                );
                assert_eq!(
                    calls[0].input.get("search").and_then(|v| v.as_str()),
                    Some("old")
                );
                assert_eq!(
                    calls[0].input.get("replace").and_then(|v| v.as_str()),
                    Some("new")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }

    #[tokio::test]
    /// 验证拿到工具结果后会给出最终回答。
    async fn test_mock_model_tool_result_response() {
        let mock = MockModelAdapter;
        let messages = vec![
            ChatMessage::User {
                content: "/ls".to_string(),
            },
            ChatMessage::AssistantToolCall {
                tool_use_id: "1".to_string(),
                tool_name: "list_files".to_string(),
                input: serde_json::json!({}),
            },
            ChatMessage::ToolResult {
                tool_use_id: "1".to_string(),
                tool_name: "list_files".to_string(),
                content: "file1.txt\nfile2.txt".to_string(),
                is_error: false,
            },
        ];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::Assistant { content, .. } => {
                assert!(content.contains("目录内容如下"));
                assert!(content.contains("file1.txt"));
            }
            _ => panic!("Expected Assistant response"),
        }
    }

    #[tokio::test]
    /// 验证未知输入会返回默认帮助文本。
    async fn test_mock_model_default_response() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "hello".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::Assistant { content, .. } => {
                assert!(content.contains("最小骨架版本"));
                assert!(content.contains("/tools"));
            }
            _ => panic!("Expected Assistant response"),
        }
    }

    #[tokio::test]
    /// 验证 `/patch` 会生成批量替换调用。
    async fn test_mock_model_patch_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/patch file.txt::old1::new1||old2::new2".to_string(),
        }];
        let result = mock.next(&messages).await.expect("next");
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "patch_file");
                assert_eq!(
                    calls[0].input.get("path").and_then(|v| v.as_str()),
                    Some("file.txt")
                );

                let replacements = calls[0]
                    .input
                    .get("replacements")
                    .and_then(|v| v.as_array())
                    .expect("replacements");
                assert_eq!(replacements.len(), 2);
                assert_eq!(
                    replacements[0].get("search").and_then(|v| v.as_str()),
                    Some("old1")
                );
                assert_eq!(
                    replacements[0].get("replace").and_then(|v| v.as_str()),
                    Some("new1")
                );
                assert_eq!(
                    replacements[1].get("search").and_then(|v| v.as_str()),
                    Some("old2")
                );
                assert_eq!(
                    replacements[1].get("replace").and_then(|v| v.as_str()),
                    Some("new2")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }
}
