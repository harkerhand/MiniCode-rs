use std::path::Path;

use crate::tool::{McpServerSummary, SkillSummary};

pub fn build_system_prompt(
    cwd: &Path,
    permission_summary: &[String],
    skills: &[SkillSummary],
    mcp_servers: &[McpServerSummary],
) -> String {
    let mut lines = vec![
        "你是 MiniCode 的编码代理，必须在当前仓库中完成用户请求。".to_string(),
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
            lines.push(format!(
                "- {} status={} tools={}",
                s.name, s.status, s.tool_count
            ));
        }
    }

    lines.join("\n")
}
