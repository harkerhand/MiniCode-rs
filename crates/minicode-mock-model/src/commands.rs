use minicode_types::{AgentStep, ToolCall};
use uuid::Uuid;

pub(crate) fn render_tool_result_response(tool_name: &str, content: &str) -> AgentStep {
    let response = match tool_name {
        "list_files" => {
            format!("目录内容如下：\n\n{}", content)
        }
        "read_file" => {
            format!("文件内容如下：\n\n{}", content)
        }
        "write_file" | "edit_file" | "patch_file" | "modify_file" => content.to_string(),
        _ => {
            format!("我拿到了工具结果：\n\n{}", content)
        }
    };
    AgentStep::Assistant {
        content: response,
        kind: Some("final".to_string()),
        diagnostics: None,
    }
}

pub(crate) fn parse_user_command(user_text: &str) -> Option<AgentStep> {
    if user_text == "/tools" {
        return Some(AgentStep::Assistant {
            content: "可用工具：ask_user, list_files, grep_files, read_file, write_file, modify_file, patch_file, edit_file, run_command, load_skill, web_fetch, web_search".to_string(),
            kind: Some("final".to_string()),
            diagnostics: None,
        });
    }

    if user_text.starts_with("/ls") {
        let dir = user_text.replace("/ls", "").trim().to_string();
        let path = if dir.is_empty() { ".".to_string() } else { dir };
        return Some(AgentStep::ToolCalls {
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
            return Some(AgentStep::ToolCalls {
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

    if user_text.starts_with("/read ") {
        let path = user_text.strip_prefix("/read ").unwrap_or("").trim();
        if !path.is_empty() {
            return Some(AgentStep::ToolCalls {
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

    if user_text.starts_with("/cmd ") {
        let payload = user_text.strip_prefix("/cmd ").unwrap_or("").trim();
        let parts: Vec<&str> = payload.split_whitespace().collect();
        if !parts.is_empty() {
            let command = parts[0].to_string();
            let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
            return Some(AgentStep::ToolCalls {
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

    if user_text.starts_with("/write ") {
        let payload = user_text.strip_prefix("/write ").unwrap_or("");
        if let Some(split_pos) = payload.find("::") {
            let path = payload[..split_pos].trim().to_string();
            let content = payload[split_pos + 2..].to_string();
            return Some(AgentStep::ToolCalls {
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
            return Some(AgentStep::Assistant {
                content: "用法: /write 路径::内容".to_string(),
                kind: Some("final".to_string()),
                diagnostics: None,
            });
        }
    }

    if user_text.starts_with("/edit ") {
        let payload = user_text.strip_prefix("/edit ").unwrap_or("");
        let parts: Vec<&str> = payload.split("::").collect();
        if parts.len() == 3 {
            let target_path = parts[0].trim().to_string();
            let search = parts[1].to_string();
            let replace = parts[2].to_string();
            return Some(AgentStep::ToolCalls {
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
            return Some(AgentStep::Assistant {
                content: "用法: /edit 路径::查找文本::替换文本".to_string(),
                kind: Some("final".to_string()),
                diagnostics: None,
            });
        }
    }

    if user_text.starts_with("/patch ") {
        let payload = user_text.strip_prefix("/patch ").unwrap_or("");
        let parts: Vec<&str> = payload.split("||").collect();
        if parts.is_empty() {
            return Some(AgentStep::Assistant {
                content: "用法: /patch 路径::查找1::替换1||查找2::替换2||...".to_string(),
                kind: Some("final".to_string()),
                diagnostics: None,
            });
        }

        let path_parts: Vec<&str> = parts[0].split("::").collect();
        if path_parts.len() < 3 {
            return Some(AgentStep::Assistant {
                content: "用法: /patch 路径::查找1::替换1||查找2::替换2||...".to_string(),
                kind: Some("final".to_string()),
                diagnostics: None,
            });
        }

        let target_path = path_parts[0].trim().to_string();
        let mut replacements = vec![];

        replacements.push(serde_json::json!({
            "search": path_parts[1].to_string(),
            "replace": path_parts[2].to_string()
        }));

        for replacement_part in &parts[1..] {
            let rep_parts: Vec<&str> = replacement_part.split("::").collect();
            if rep_parts.len() >= 2 {
                replacements.push(serde_json::json!({
                    "search": rep_parts[0].to_string(),
                    "replace": rep_parts.get(1).map(|s| s.to_string()).unwrap_or_default()
                }));
            }
        }

        return Some(AgentStep::ToolCalls {
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

    None
}

pub(crate) fn default_response() -> AgentStep {
    AgentStep::Assistant {
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
    }
}
