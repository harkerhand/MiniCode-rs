use std::collections::HashSet;
use std::path::{Path, PathBuf};

use minicode_config::runtime_store;
use minicode_permissions::get_permission_manager;
use minicode_tool::get_tool_registry;

/// 每个文件的最大字符数
const MAX_PER_FILE_CHARS: usize = 8_000;

/// 总字符数限制
const MAX_TOTAL_CHARS: usize = 20_000;

/// 每个目录的候选文件名
const CANDIDATES_PER_DIR: &[&str] = &[
    "MINI.md",
    "MINI.local.md",
    ".mini-code/MINI.md",
    "CLAUDE.md",
    "CLAUDE.local.md",
    ".claude/CLAUDE.md",
];

/// 尝试读取文件内容，失败时返回 `None`。
fn maybe_read(path: impl AsRef<Path>) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 内容哈希用于去重
fn content_hash(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

/// 截断文本到限制
fn truncate_to(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= limit {
        trimmed.to_string()
    } else {
        format!("{}\n\n[truncated]", &trimmed[..limit])
    }
}

/// 发现规则目录中的 .md 文件
fn discover_rule_files(rules_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(rules_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "md") {
                files.push(path);
            }
        }
        files.sort();
    }
    files
}

/// 发现指令文件
fn discover_instruction_files(cwd: &Path, home_dir: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let mut seen_hashes = HashSet::new();

    // 1. 全局指令文件
    let global_candidates = [home_dir.join("MINI.md"), home_dir.join("CLAUDE.md")];
    for candidate in &global_candidates {
        if let Some(content) = maybe_read(candidate) {
            let hash = content_hash(&content);
            if seen_hashes.insert(hash) {
                files.push((candidate.clone(), content));
            }
            break; // 只取一个全局文件
        }
    }

    // 2. 全局规则目录
    let global_rules_dir = home_dir.join("rules");
    for rule_path in discover_rule_files(&global_rules_dir) {
        if let Some(content) = maybe_read(&rule_path) {
            let hash = content_hash(&content);
            if seen_hashes.insert(hash) {
                files.push((rule_path, content));
            }
        }
    }

    // 3. 从 cwd 向上遍历祖先目录
    let mut current = Some(cwd);
    while let Some(dir) = current {
        for name in CANDIDATES_PER_DIR {
            let candidate = dir.join(name);
            if let Some(content) = maybe_read(&candidate) {
                let hash = content_hash(&content);
                if seen_hashes.insert(hash) {
                    files.push((candidate, content));
                }
            }
        }

        // 项目规则目录
        let rules_dir = dir.join(".mini-code").join("rules");
        for rule_path in discover_rule_files(&rules_dir) {
            if let Some(content) = maybe_read(&rule_path) {
                let hash = content_hash(&content);
                if seen_hashes.insert(hash) {
                    files.push((rule_path, content));
                }
            }
        }

        current = dir.parent();
    }

    files
}

/// 加载 Memory 内容
fn load_memory(cwd: &Path, home_dir: &Path) -> String {
    let files = discover_instruction_files(cwd, home_dir);
    if files.is_empty() {
        return String::new();
    }

    let mut sections = vec!["# Instructions".to_string()];
    let mut remaining = MAX_TOTAL_CHARS;

    for (path, content) in &files {
        if remaining == 0 {
            sections.push(
                "_Additional instruction content omitted after reaching the prompt budget._"
                    .to_string(),
            );
            break;
        }

        let truncated = truncate_to(content, MAX_PER_FILE_CHARS.min(remaining));
        remaining = remaining.saturating_sub(truncated.len());

        let scope = if path.to_string_lossy().contains("/rules/") {
            "rules"
        } else if path.starts_with(home_dir) {
            "global"
        } else {
            "project"
        };

        sections.push(format!(
            "## {} (scope: {})\n\n{}",
            path.display(),
            scope,
            truncated
        ));
    }

    sections.join("\n\n")
}

/// 渲染 Memory 报告（用于 /memory 命令）
pub fn render_memory_report(cwd: &Path, home_dir: &Path) -> String {
    let files = discover_instruction_files(cwd, home_dir);
    if files.is_empty() {
        return "No memory files loaded.".to_string();
    }

    let mut report = vec![format!("Memory files loaded: {}", files.len())];

    for (i, (path, content)) in files.iter().enumerate() {
        let scope = if path.to_string_lossy().contains("/rules/") {
            "rules"
        } else if path.starts_with(home_dir) {
            "global"
        } else {
            "project"
        };

        let lines = content.trim().lines().count();
        let chars = content.len();
        let preview = content.trim().lines().next().unwrap_or("<empty>");

        report.push(format!(
            "{}. {}\n   scope: {}\n   lines: {}\n   chars: {}\n   preview: {}",
            i + 1,
            path.display(),
            scope,
            lines,
            chars,
            preview
        ));
    }

    report.join("\n\n")
}

/// 组合运行上下文、权限、技能和 MCP 信息，生成系统提示词。
pub fn build_system_prompt() -> String {
    let cwd = runtime_store().cwd.clone();
    let permission_summary = get_permission_manager().get_summary_text();
    let skills = get_tool_registry().get_skills();
    let mcp_servers = get_tool_registry().get_mcp_servers();

    let mut lines = Vec::new();
    lines.push(format!(include_str!("./prompt.txt"), cwd.display()));

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

    // 加载 Memory 内容（MINI.md、规则目录等）
    if let Some(home) = dirs::home_dir() {
        let memory_content = load_memory(&cwd, &home);
        if !memory_content.is_empty() {
            lines.push(memory_content);
        }
    }

    lines.join("\n\n")
}
