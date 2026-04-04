use std::path::Path;

use crate::tool::{McpServerSummary, SkillSummary};

fn maybe_read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

pub fn build_system_prompt(
    cwd: &Path,
    permission_summary: &[String],
    skills: &[SkillSummary],
    mcp_servers: &[McpServerSummary],
) -> String {
    let mut lines = vec![
        "你是 MiniCode 的编码代理，必须在当前仓库中完成用户请求。".to_string(),
        "默认行为：先检查仓库并优先调用工具执行，再给结论；不要只停留在理论建议。".to_string(),
        "如果用户明确要求你修改/生成/修复内容，直接动手执行，不要只给计划。".to_string(),
        "优先使用读文件、搜索、编辑和验证命令来解决问题，而不是只给理论建议。".to_string(),
        "调用工具时必须提供该工具 input_schema 中的必填字段；缺少必填字段时不要发起调用。"
            .to_string(),
        "当工具返回参数错误（例如 path is required）时，先修正参数再重试，不要重复同一个无效输入。"
            .to_string(),
        "读取文件时，read_file 必须提供 path；若路径不确定，先 list_files 或 grep_files 定位。"
            .to_string(),
        "确实需要补充信息时，调用 ask_user 提一个简短问题并等待用户回复。".to_string(),
        "结构化响应协议：未完成且会继续调用工具时，用 <progress> 开头；仅在任务完成可交还控制时，用 <final> 开头。"
            .to_string(),
        "发出 <progress> 后不要停止，下一步应继续执行工具调用或代码修改。".to_string(),
        format!("当前工作目录: {}", cwd.display()),
        "权限摘要:".to_string(),
    ];
    for item in permission_summary {
        lines.push(format!("- {item}"));
    }

    lines.push("可用技能:".to_string());
    if skills.is_empty() {
        lines.push("- (none)".to_string());
    } else {
        for skill in skills {
            lines.push(format!("- {}: {}", skill.name, skill.description));
        }
    }

    lines.push("MCP 服务:".to_string());
    if mcp_servers.is_empty() {
        lines.push("- (none)".to_string());
    } else {
        for s in mcp_servers {
            let resources = s
                .resource_count
                .map(|x| format!(", resources={x}"))
                .unwrap_or_default();
            let prompts = s
                .prompt_count
                .map(|x| format!(", prompts={x}"))
                .unwrap_or_default();
            let protocol = s
                .protocol
                .as_ref()
                .map(|x| format!(", protocol={x}"))
                .unwrap_or_default();
            let suffix = s
                .error
                .as_ref()
                .map(|x| format!(" ({x})"))
                .unwrap_or_default();
            lines.push(format!(
                "- {}: {}, tools={}{}{}{}{}",
                s.name, s.status, s.tool_count, resources, prompts, protocol, suffix
            ));
        }

        if mcp_servers.iter().any(|s| s.status == "connected") {
            lines.push("已连接的 MCP 工具会以 mcp__server__tool 形式出现在工具列表中；若服务支持资源或提示词，请优先使用相关 MCP 工具。".to_string());
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global_path = home.join(".claude").join("CLAUDE.md");
        if let Some(content) = maybe_read(&global_path) {
            lines.push(format!(
                "全局指令（{}）:\n{}",
                global_path.display(),
                content
            ));
        }
    }

    let project_path = cwd.join("CLAUDE.md");
    if let Some(content) = maybe_read(&project_path) {
        lines.push(format!(
            "项目指令（{}）:\n{}",
            project_path.display(),
            content
        ));
    }

    lines.join("\n")
}
