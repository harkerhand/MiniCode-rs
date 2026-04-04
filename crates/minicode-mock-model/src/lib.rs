use anyhow::Result;
use async_trait::async_trait;
use minicode_types::{AgentStep, ChatMessage, ModelAdapter, ToolCall};
use uuid::Uuid;

pub struct MockModelAdapter;

/// 取最近一条用户消息文本。
fn last_user_message(messages: &[ChatMessage]) -> String {
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
fn last_tool_message(messages: &[ChatMessage]) -> Option<(String, String, String)> {
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
fn extract_latest_assistant_call(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| match m {
        ChatMessage::AssistantToolCall { tool_name, .. } => Some(tool_name.clone()),
        _ => None,
    })
}

#[async_trait]
impl ModelAdapter for MockModelAdapter {
    /// 根据简单规则返回工具调用或最终回复，用于本地测试。
    async fn next(&self, messages: &[ChatMessage]) -> Result<AgentStep> {
        // 阶段 1: 如果有工具结果，根据前一个工具调用来生成响应
        if let Some((_, _, content)) = last_tool_message(messages)
            && let Some(tool_name) = extract_latest_assistant_call(messages)
        {
            let response = match tool_name.as_str() {
                "list_files" => {
                    format!("目录内容如下：\n\n{}", content)
                }
                "read_file" => {
                    format!("文件内容如下：\n\n{}", content)
                }
                "write_file" | "edit_file" | "patch_file" | "modify_file" => content,
                _ => {
                    format!("我拿到了工具结果：\n\n{}", content)
                }
            };
            return Ok(AgentStep::Assistant {
                content: response,
                kind: Some("final".to_string()),
                diagnostics: None,
            });
        }

        // 阶段 2: 解析用户命令
        let user_text = last_user_message(messages).trim().to_string();

        // 列出可用工具
        if user_text == "/tools" {
            return Ok(AgentStep::Assistant {
                content: "可用工具：ask_user, list_files, grep_files, read_file, write_file, modify_file, patch_file, edit_file, run_command, load_skill".to_string(),
                kind: Some("final".to_string()),
                diagnostics: None,
            });
        }

        // 列出文件
        if user_text.starts_with("/ls") {
            let dir = user_text.replace("/ls", "").trim().to_string();
            let path = if dir.is_empty() { ".".to_string() } else { dir };
            return Ok(AgentStep::ToolCalls {
                calls: vec![ToolCall {
                    id: Uuid::new_v4().to_string(),
                    tool_name: "list_files".to_string(),
                    input: serde_json::json!({ "path": path }),
                }],
                content: None,
                content_kind: None,
                diagnostics: None,
            });
        }

        // 搜索文件
        if user_text.starts_with("/grep ") {
            let payload = user_text.strip_prefix("/grep ").unwrap_or("").trim();
            let parts: Vec<&str> = payload.split("::").collect();
            let pattern = parts.first().map(|s| s.trim()).unwrap_or("").to_string();
            let search_path = parts.get(1).map(|s| s.trim().to_string());

            if !pattern.is_empty() {
                let mut input = serde_json::json!({ "pattern": pattern });
                if let Some(path) = search_path {
                    input["path"] = serde_json::json!(path);
                }
                return Ok(AgentStep::ToolCalls {
                    calls: vec![ToolCall {
                        id: Uuid::new_v4().to_string(),
                        tool_name: "grep_files".to_string(),
                        input,
                    }],
                    content: None,
                    content_kind: None,
                    diagnostics: None,
                });
            }
        }

        // 读取文件
        if user_text.starts_with("/read ") {
            let path = user_text.strip_prefix("/read ").unwrap_or("").trim();
            if !path.is_empty() {
                return Ok(AgentStep::ToolCalls {
                    calls: vec![ToolCall {
                        id: Uuid::new_v4().to_string(),
                        tool_name: "read_file".to_string(),
                        input: serde_json::json!({ "path": path }),
                    }],
                    content: None,
                    content_kind: None,
                    diagnostics: None,
                });
            }
        }

        // 执行命令
        if user_text.starts_with("/cmd ") {
            let payload = user_text.strip_prefix("/cmd ").unwrap_or("").trim();
            let parts: Vec<&str> = payload.split_whitespace().collect();
            if !parts.is_empty() {
                let command = parts[0].to_string();
                let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
                return Ok(AgentStep::ToolCalls {
                    calls: vec![ToolCall {
                        id: Uuid::new_v4().to_string(),
                        tool_name: "run_command".to_string(),
                        input: serde_json::json!({ "command": command, "args": args }),
                    }],
                    content: None,
                    content_kind: None,
                    diagnostics: None,
                });
            }
        }

        // 写入文件
        if user_text.starts_with("/write ") {
            let payload = user_text.strip_prefix("/write ").unwrap_or("");
            if let Some(split_pos) = payload.find("::") {
                let path = payload[..split_pos].trim().to_string();
                let content = payload[split_pos + 2..].to_string();
                return Ok(AgentStep::ToolCalls {
                    calls: vec![ToolCall {
                        id: Uuid::new_v4().to_string(),
                        tool_name: "write_file".to_string(),
                        input: serde_json::json!({ "path": path, "content": content }),
                    }],
                    content: None,
                    content_kind: None,
                    diagnostics: None,
                });
            } else {
                return Ok(AgentStep::Assistant {
                    content: "用法: /write 路径::内容".to_string(),
                    kind: Some("final".to_string()),
                    diagnostics: None,
                });
            }
        }

        // 编辑文件
        if user_text.starts_with("/edit ") {
            let payload = user_text.strip_prefix("/edit ").unwrap_or("");
            let parts: Vec<&str> = payload.split("::").collect();
            if parts.len() == 3 {
                let target_path = parts[0].trim().to_string();
                let search = parts[1].to_string();
                let replace = parts[2].to_string();
                return Ok(AgentStep::ToolCalls {
                    calls: vec![ToolCall {
                        id: Uuid::new_v4().to_string(),
                        tool_name: "edit_file".to_string(),
                        input: serde_json::json!({
                            "path": target_path,
                            "search": search,
                            "replace": replace
                        }),
                    }],
                    content: None,
                    content_kind: None,
                    diagnostics: None,
                });
            } else {
                return Ok(AgentStep::Assistant {
                    content: "用法: /edit 路径::查找文本::替换文本".to_string(),
                    kind: Some("final".to_string()),
                    diagnostics: None,
                });
            }
        }

        // 补丁编辑（多个替换）
        if user_text.starts_with("/patch ") {
            let payload = user_text.strip_prefix("/patch ").unwrap_or("");
            let parts: Vec<&str> = payload.split("||").collect();
            if parts.is_empty() {
                return Ok(AgentStep::Assistant {
                    content: "用法: /patch 路径::查找1::替换1||查找2::替换2||...".to_string(),
                    kind: Some("final".to_string()),
                    diagnostics: None,
                });
            }

            let path_parts: Vec<&str> = parts[0].split("::").collect();
            if path_parts.len() < 3 {
                return Ok(AgentStep::Assistant {
                    content: "用法: /patch 路径::查找1::替换1||查找2::替换2||...".to_string(),
                    kind: Some("final".to_string()),
                    diagnostics: None,
                });
            }

            let target_path = path_parts[0].trim().to_string();
            let mut replacements = vec![];

            // 第一个替换
            replacements.push(serde_json::json!({
                "search": path_parts[1].to_string(),
                "replace": path_parts[2].to_string()
            }));

            // 后续替换
            for replacement_part in &parts[1..] {
                let rep_parts: Vec<&str> = replacement_part.split("::").collect();
                if rep_parts.len() >= 2 {
                    replacements.push(serde_json::json!({
                        "search": rep_parts[0].to_string(),
                        "replace": rep_parts.get(1).map(|s| s.to_string()).unwrap_or_default()
                    }));
                }
            }

            return Ok(AgentStep::ToolCalls {
                calls: vec![ToolCall {
                    id: Uuid::new_v4().to_string(),
                    tool_name: "patch_file".to_string(),
                    input: serde_json::json!({
                        "path": target_path,
                        "replacements": replacements
                    }),
                }],
                content: None,
                content_kind: None,
                diagnostics: None,
            });
        }

        // 阶段 3: 默认提示
        Ok(AgentStep::Assistant {
            content: [
                "这是一个最小骨架版本。",
                "你可以试试：",
                "/tools",
                "/ls",
                "/grep pattern::src",
                "/read README.md",
                "/cmd pwd",
                "/write notes.txt::hello",
                "/edit notes.txt::hello::hello world",
                "/patch file.txt::old1::new1||old2::new2",
            ]
            .join("\n"),
            kind: Some("final".to_string()),
            diagnostics: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    /// 验证 `/tools` 返回工具清单文本。
    async fn test_mock_model_tools_command() {
        let mock = MockModelAdapter;
        let messages = vec![ChatMessage::User {
            content: "/tools".to_string(),
        }];
        let result = mock.next(&messages).await.unwrap();
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
        let result = mock.next(&messages).await.unwrap();
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "list_files");
                assert_eq!(calls[0].input.get("path").unwrap().as_str(), Some("src"));
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
        let result = mock.next(&messages).await.unwrap();
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "grep_files");
                assert_eq!(
                    calls[0].input.get("pattern").unwrap().as_str(),
                    Some("fn main")
                );
                assert_eq!(calls[0].input.get("path").unwrap().as_str(), Some("src"));
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
        let result = mock.next(&messages).await.unwrap();
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "read_file");
                assert_eq!(
                    calls[0].input.get("path").unwrap().as_str(),
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
        let result = mock.next(&messages).await.unwrap();
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "write_file");
                assert_eq!(
                    calls[0].input.get("path").unwrap().as_str(),
                    Some("notes.txt")
                );
                assert_eq!(
                    calls[0].input.get("content").unwrap().as_str(),
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
        let result = mock.next(&messages).await.unwrap();
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "edit_file");
                assert_eq!(
                    calls[0].input.get("path").unwrap().as_str(),
                    Some("file.txt")
                );
                assert_eq!(calls[0].input.get("search").unwrap().as_str(), Some("old"));
                assert_eq!(calls[0].input.get("replace").unwrap().as_str(), Some("new"));
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
        let result = mock.next(&messages).await.unwrap();
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
        let result = mock.next(&messages).await.unwrap();
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
        let result = mock.next(&messages).await.unwrap();
        match result {
            AgentStep::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "patch_file");
                assert_eq!(
                    calls[0].input.get("path").unwrap().as_str(),
                    Some("file.txt")
                );

                let replacements = calls[0]
                    .input
                    .get("replacements")
                    .unwrap()
                    .as_array()
                    .unwrap();
                assert_eq!(replacements.len(), 2);
                assert_eq!(
                    replacements[0].get("search").unwrap().as_str(),
                    Some("old1")
                );
                assert_eq!(
                    replacements[0].get("replace").unwrap().as_str(),
                    Some("new1")
                );
                assert_eq!(
                    replacements[1].get("search").unwrap().as_str(),
                    Some("old2")
                );
                assert_eq!(
                    replacements[1].get("replace").unwrap().as_str(),
                    Some("new2")
                );
            }
            _ => panic!("Expected ToolCalls"),
        }
    }
}
