#[derive(Debug, Clone)]
pub struct LocalToolShortcut {
    pub tool_name: &'static str,
    pub input: serde_json::Value,
}

/// 解析本地斜杠命令并映射为内置工具调用。
pub fn parse_local_tool_shortcut(input: &str) -> Option<LocalToolShortcut> {
    if input == "/ls" {
        return Some(LocalToolShortcut {
            tool_name: "list_files",
            input: serde_json::json!({ "path": "." }),
        });
    }

    if let Some(path) = input.strip_prefix("/ls ") {
        return Some(LocalToolShortcut {
            tool_name: "list_files",
            input: serde_json::json!({ "path": path.trim() }),
        });
    }

    if let Some(rest) = input.strip_prefix("/grep ") {
        let parts = rest.split("::").collect::<Vec<_>>();
        if parts.len() == 2 {
            if parts[0].trim().is_empty() {
                return None;
            }
            return Some(LocalToolShortcut {
                tool_name: "grep_files",
                input: serde_json::json!({ "pattern": parts[0].trim(), "path": parts[1].trim() }),
            });
        }
        if rest.trim().is_empty() {
            return None;
        }
        return Some(LocalToolShortcut {
            tool_name: "grep_files",
            input: serde_json::json!({ "pattern": rest.trim() }),
        });
    }

    if let Some(path) = input.strip_prefix("/read ") {
        if path.trim().is_empty() {
            return None;
        }
        return Some(LocalToolShortcut {
            tool_name: "read_file",
            input: serde_json::json!({ "path": path.trim() }),
        });
    }

    if let Some(rest) = input.strip_prefix("/write ") {
        let parts = rest.splitn(2, "::").collect::<Vec<_>>();
        if parts.len() == 2 {
            return Some(LocalToolShortcut {
                tool_name: "write_file",
                input: serde_json::json!({ "path": parts[0].trim(), "content": parts[1] }),
            });
        }
        return None;
    }

    if let Some(rest) = input.strip_prefix("/modify ") {
        let parts = rest.splitn(2, "::").collect::<Vec<_>>();
        if parts.len() == 2 {
            return Some(LocalToolShortcut {
                tool_name: "modify_file",
                input: serde_json::json!({ "path": parts[0].trim(), "content": parts[1] }),
            });
        }
        return None;
    }

    if let Some(rest) = input.strip_prefix("/edit ") {
        let parts = rest.splitn(3, "::").collect::<Vec<_>>();
        if parts.len() == 3 {
            return Some(LocalToolShortcut {
                tool_name: "edit_file",
                input: serde_json::json!({ "path": parts[0].trim(), "search": parts[1], "replace": parts[2] }),
            });
        }
        return None;
    }

    if let Some(rest) = input.strip_prefix("/patch ") {
        let parts = rest.split("::").collect::<Vec<_>>();
        if parts.len() >= 3 && parts.len() % 2 == 1 {
            let path = parts[0].trim();
            if path.is_empty() {
                return None;
            }
            let mut replacements = Vec::new();
            let mut i = 1;
            while i + 1 < parts.len() {
                replacements.push(serde_json::json!({
                    "search": parts[i],
                    "replace": parts[i + 1],
                }));
                i += 2;
            }
            return Some(LocalToolShortcut {
                tool_name: "patch_file",
                input: serde_json::json!({ "path": path, "replacements": replacements }),
            });
        }
        return None;
    }

    if let Some(rest) = input.strip_prefix("/cmd ") {
        if let Some((cwd, cmd)) = rest.split_once("::") {
            if cmd.trim().is_empty() {
                return None;
            }
            return Some(LocalToolShortcut {
                tool_name: "run_command",
                input: serde_json::json!({ "cwd": cwd.trim(), "command": cmd.trim() }),
            });
        }

        if rest.trim().is_empty() {
            return None;
        }
        return Some(LocalToolShortcut {
            tool_name: "run_command",
            input: serde_json::json!({ "command": rest.trim() }),
        });
    }

    None
}
