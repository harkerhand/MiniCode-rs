use std::path::Path;

use minicode_config::runtime_store;
use minicode_permissions::get_permission_manager;
use minicode_types::{McpServerSummary, SkillSummary};

/// 尝试读取文件内容，失败时返回 `None`。
fn maybe_read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// 组合运行上下文、权限、技能和 MCP 信息，生成系统提示词。
pub fn build_system_prompt(skills: &[SkillSummary], mcp_servers: &[McpServerSummary]) -> String {
    let cwd = runtime_store().cwd.clone();
    let permission_summary = get_permission_manager().get_summary_text();

    let mut lines = vec![
        "You are mini-code, a terminal coding assistant.".to_string(),
        "Default behavior: inspect the repository, use tools, make code changes when appropriate, and explain results clearly.".to_string(),
        "Prefer reading files, searching code, editing files, and running verification commands over giving purely theoretical advice.".to_string(),
        format!("Current cwd: {}", cwd.display()),
        "You can inspect or modify paths outside the current cwd when the user asks, but tool permissions may pause for approval first.".to_string(),
        "When making code changes, keep them minimal, practical, and working-oriented.".to_string(),
        "If the user clearly asked you to build, modify, optimize, or generate something, do the work instead of stopping at a plan.".to_string(),
        "If you need user clarification, call the ask_user tool with one concise question and wait for the user reply. Do not ask clarifying questions as plain assistant text.".to_string(),
        "Do not choose subjective preferences such as colors, visual style, copy tone, or naming unless the user explicitly told you to decide yourself.".to_string(),
        "When using read_file, pay attention to the header fields. If it says TRUNCATED: yes, continue reading with a larger offset before concluding that the file itself is cut off.".to_string(),
        "If the user names a skill or clearly asks for a workflow that matches a listed skill, call load_skill before following it.".to_string(),
        "Structured response protocol:".to_string(),
        "- When you are still working and will continue with more tool calls, start your text with <progress>.".to_string(),
        "- Only when the task is actually complete and you are ready to hand control back, start your text with <final>.".to_string(),
        "- Use ask_user when clarification is required; that tool ends the turn and waits for user input.".to_string(),
        "- Do not stop after a progress update. After a <progress> message, continue the task in the next step.".to_string(),
        "- Plain assistant text without <progress> is treated as a completed assistant message for this turn.".to_string(),
    ];

    if !permission_summary.is_empty() {
        lines.push(format!(
            "Permission context:\n{}",
            permission_summary.join("\n")
        ));
    }

    if skills.is_empty() {
        lines.push("Available skills:\n- none discovered".to_string());
    } else {
        let skills_text = skills
            .iter()
            .map(|skill| format!("- {}: {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n");
        lines.push(format!("Available skills:\n{}", skills_text));
    }

    if !mcp_servers.is_empty() {
        let servers_text = mcp_servers
            .iter()
            .map(|server| {
                let suffix = server
                    .error
                    .as_ref()
                    .map(|x| format!(" ({})", x))
                    .unwrap_or_default();
                let protocol = server
                    .protocol
                    .as_ref()
                    .map(|x| format!(", protocol={}", x))
                    .unwrap_or_default();
                let resources = server
                    .resource_count
                    .map(|x| format!(", resources={}", x))
                    .unwrap_or_default();
                let prompts = server
                    .prompt_count
                    .map(|x| format!(", prompts={}", x))
                    .unwrap_or_default();
                format!(
                    "- {}: {}, tools={}{}{}{}{}",
                    server.name,
                    server.status,
                    server.tool_count,
                    resources,
                    prompts,
                    protocol,
                    suffix
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        lines.push(format!("Configured MCP servers:\n{}", servers_text));

        if mcp_servers.iter().any(|s| s.status == "connected") {
            lines.push(
                "Connected MCP tools are already exposed in the tool list with names prefixed like mcp__server__tool. Use list_mcp_resources/read_mcp_resource and list_mcp_prompts/get_mcp_prompt when a server exposes those capabilities.".to_string(),
            );
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global_path = home.join(".claude").join("CLAUDE.md");
        if let Some(content) = maybe_read(&global_path) {
            lines.push(format!(
                "Global instructions from ~/.claude/CLAUDE.md:\n{}",
                content
            ));
        }
    }

    let project_path = cwd.join("CLAUDE.md");
    if let Some(content) = maybe_read(&project_path) {
        lines.push(format!(
            "Project instructions from {}:\n{}",
            project_path.display(),
            content
        ));
    }

    lines.join("\n\n")
}
