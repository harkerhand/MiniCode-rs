use std::process::Stdio;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use minicode_background_tasks::register_background_shell_task;
use minicode_config::RuntimeConfig;
use minicode_file_review::apply_reviewed_file_change;
use minicode_mcp::{create_mcp_backed_tools, set_mcp_logging_enabled};
use minicode_permissions::EnsureCommandOptions;
use minicode_skills::{discover_skills, load_skill};
use minicode_tool::{Tool, ToolContext, ToolRegistry, ToolResult};
use minicode_workspace::resolve_tool_path;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;

const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) MiniCode/0.1";

/// 控制 MCP 启动阶段日志开关。
pub fn set_mcp_startup_logging_enabled(enabled: bool) {
    set_mcp_logging_enabled(enabled);
}

#[derive(Default)]
pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "ask_user"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "向用户提问并暂停当前轮次。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"question":{"type":"string"}},"required":["question"]})
    }
    /// 透传问题并要求当前轮等待用户回复。
    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let question = input
            .get("question")
            .and_then(|x| x.as_str())
            .unwrap_or("请补充信息")
            .to_string();
        ToolResult {
            ok: true,
            output: question,
            background_task: None,
            await_user: true,
        }
    }
}

#[derive(Default)]
pub struct ListFilesTool;
#[async_trait]
impl Tool for ListFilesTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "list_files"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "列出目录内容（最多200条）。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
    /// 列出目标目录中的文件和子目录。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or(".");
        let target = match resolve_tool_path(context, path, "list").await {
            Ok(p) => p,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let entries = match std::fs::read_dir(target) {
            Ok(x) => x,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let mut lines = vec![];
        for entry in entries.take(200).flatten() {
            let prefix = if entry.file_type().map(|f| f.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            lines.push(format!(
                "{} {}",
                prefix,
                entry.file_name().to_string_lossy()
            ));
        }
        ToolResult::ok(if lines.is_empty() {
            "(empty)".to_string()
        } else {
            lines.join("\n")
        })
    }
}

#[derive(Default)]
pub struct GrepFilesTool;
#[derive(Debug, Deserialize)]
struct GrepInput {
    pattern: String,
    path: Option<String>,
}
#[async_trait]
impl Tool for GrepFilesTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "grep_files"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "Search text using ripgrep, with results limited to first 100 matches for performance."
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"]})
    }
    /// 使用 `rg` 搜索文本并返回匹配结果。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let parsed: GrepInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let mut args = vec!["-n".to_string(), "--no-heading".to_string(), parsed.pattern];
        if let Some(path) = parsed.path {
            let p = match resolve_tool_path(context, &path, "search").await {
                Ok(v) => v,
                Err(err) => return ToolResult::err(err.to_string()),
            };
            args.push(p.to_string_lossy().to_string());
        } else {
            args.push(".".to_string());
        }

        match Command::new("rg")
            .args(args)
            .current_dir(&context.cwd)
            .output()
            .await
        {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let text = if stdout.is_empty() && stderr.is_empty() {
                    "(no matches)".to_string()
                } else if stdout.is_empty() {
                    stderr
                } else if stderr.is_empty() {
                    stdout
                } else {
                    format!("{}\n{}", stdout, stderr)
                };

                // Check if output might be truncated and add indicator
                let result_lines_count = text.lines().count();
                let final_text = if result_lines_count >= 100 {
                    format!(
                        "{}\n\n[Results limited to first 100 matches. Refine your search pattern for more specific results.]",
                        text
                    )
                } else {
                    text
                };
                ToolResult::ok(final_text)
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Default)]
pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "read_file"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "Read UTF-8 text file with optional offset/limit for chunked reading. Check TRUNCATED header."
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["path"]})
    }
    /// 分块读取 UTF-8 文件并带上截断头信息。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("path is required");
        }
        let offset = input.get("offset").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|x| x.as_u64())
            .unwrap_or(8000)
            .min(20_000) as usize;

        let target = match resolve_tool_path(context, path, "read").await {
            Ok(p) => p,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let content = match std::fs::read_to_string(target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let chars = content.chars().collect::<Vec<_>>();
        let total_chars = chars.len();
        let safe_offset = offset.min(total_chars);
        let end = safe_offset.saturating_add(limit).min(total_chars);
        let chunk = chars[safe_offset..end].iter().collect::<String>();
        let truncated = end < total_chars;
        let header = format!(
            "FILE: {}\nOFFSET: {}\nEND: {}\nTOTAL_CHARS: {}\nTRUNCATED: {}\n\n",
            path,
            safe_offset,
            end,
            total_chars,
            if truncated {
                format!("yes - call read_file again with offset {}", end)
            } else {
                "no".to_string()
            }
        );

        ToolResult::ok(format!("{}{}", header, chunk))
    }
}

#[derive(Default)]
pub struct WriteLikeTool {
    name: &'static str,
    description: &'static str,
}
#[async_trait]
impl Tool for WriteLikeTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        self.name
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        self.description
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    /// 写入或整体替换文件内容（带权限审阅）。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|x| x.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("path is required");
        }

        let target = match resolve_tool_path(context, path, "write").await {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        match apply_reviewed_file_change(context, path, &target, content).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Default)]
pub struct EditFileTool;
#[async_trait]
impl Tool for EditFileTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "edit_file"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "Apply line-by-line edits to files using precise search/replace patterns."
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"search":{"type":"string"},"replace":{"type":"string"},"replaceAll":{"type":"boolean"}},"required":["path","search","replace"]})
    }
    /// 执行单次或全量字符串替换编辑。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let search = input.get("search").and_then(|x| x.as_str()).unwrap_or("");
        let replace = input.get("replace").and_then(|x| x.as_str()).unwrap_or("");
        let replace_all = input
            .get("replaceAll")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if path.is_empty() || search.is_empty() {
            return ToolResult::err("path/search is required");
        }

        let target = match resolve_tool_path(context, path, "write").await {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let original = match std::fs::read_to_string(&target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        if !original.contains(search) {
            return ToolResult::err(format!("Text not found in {path}"));
        }

        let next = if replace_all {
            original.replace(search, replace)
        } else {
            original.replacen(search, replace, 1)
        };

        match apply_reviewed_file_change(context, path, &target, &next).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Default)]
pub struct PatchFileTool;
#[derive(Debug, Deserialize)]
struct Replacement {
    search: String,
    replace: String,
    #[serde(rename = "replaceAll")]
    replace_all: Option<bool>,
}
#[async_trait]
impl Tool for PatchFileTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "patch_file"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "对单文件执行批量替换。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"replacements":{"type":"array","items":{"type":"object","properties":{"search":{"type":"string"},"replace":{"type":"string"},"replaceAll":{"type":"boolean"}},"required":["search","replace"]}}},"required":["path","replacements"]})
    }
    /// 依次应用多组查找替换规则。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let replacements: Vec<Replacement> = match input.get("replacements").cloned() {
            Some(v) => serde_json::from_value(v).unwrap_or_default(),
            None => vec![],
        };
        if path.is_empty() || replacements.is_empty() {
            return ToolResult::err("path/replacements is required");
        }

        let target = match resolve_tool_path(context, path, "write").await {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let mut content = match std::fs::read_to_string(&target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        for (idx, rep) in replacements.iter().enumerate() {
            if !content.contains(&rep.search) {
                return ToolResult::err(format!("Replacement {} failed: text not found", idx + 1));
            }
            if rep.replace_all.unwrap_or(false) {
                content = content.replace(&rep.search, &rep.replace);
            } else {
                content = content.replacen(&rep.search, &rep.replace, 1);
            }
        }

        match apply_reviewed_file_change(context, path, &target, &content).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Default)]
pub struct LoadSkillTool {
    cwd: std::path::PathBuf,
}
impl LoadSkillTool {
    /// 创建技能加载工具并绑定工作目录。
    pub fn new(cwd: std::path::PathBuf) -> Self {
        Self { cwd }
    }
}
#[async_trait]
impl Tool for LoadSkillTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "load_skill"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "读取某个技能的 SKILL.md 内容。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})
    }
    /// 读取指定技能的 SKILL.md 内容。
    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let name = input.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if name.is_empty() {
            return ToolResult::err("name is required");
        }
        if let Some(skill) = load_skill(&self.cwd, name) {
            return ToolResult::ok(skill.content);
        }
        ToolResult::err(format!("Skill not found: {name}"))
    }
}

#[derive(Default)]
pub struct RunCommandTool;
#[derive(Debug, Deserialize)]
struct RunCommandInput {
    command: String,
    args: Option<Vec<String>>,
    cwd: Option<String>,
}

/// 解析命令行字符串为命令与参数列表。
fn split_command_line(command_line: &str) -> Vec<String> {
    shell_words::split(command_line).unwrap_or_else(|_| {
        command_line
            .split_whitespace()
            .map(str::to_string)
            .collect()
    })
}

/// 判断输入是否为需要 shell 执行的片段。
fn looks_like_shell_snippet(command: &str, args: &[String]) -> bool {
    if !args.is_empty() {
        return false;
    }
    command.chars().any(|c| "|&;<>()$`".contains(c))
}

/// 判断命令是否在允许集合中。
fn is_allowed_command(command: &str) -> bool {
    is_read_only_command(command)
        || matches!(
            command,
            "git" | "npm" | "node" | "python3" | "pytest" | "bash" | "sh" | "bun"
        )
}

/// 判断命令是否属于只读命令。
fn is_read_only_command(command: &str) -> bool {
    matches!(
        command,
        "pwd"
            | "ls"
            | "find"
            | "rg"
            | "grep"
            | "cat"
            | "head"
            | "tail"
            | "wc"
            | "sed"
            | "echo"
            | "df"
            | "du"
            | "free"
            | "uname"
            | "uptime"
            | "whoami"
    )
}

/// 判断命令是否是后台 shell 片段。
fn is_background_shell_snippet(command: &str, args: &[String]) -> bool {
    if !args.is_empty() {
        return false;
    }
    let t = command.trim();
    t.ends_with('&') && !t.ends_with("&&")
}

#[async_trait]
impl Tool for RunCommandTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "run_command"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "运行常见开发命令。支持通过 command 传入完整 shell 片段。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"}},"required":["command"]})
    }
    /// 执行本地命令，支持权限审批和后台运行。
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let parsed: RunCommandInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let effective_cwd = if let Some(cwd) = parsed.cwd {
            match resolve_tool_path(context, &cwd, "list").await {
                Ok(v) => v,
                Err(err) => return ToolResult::err(err.to_string()),
            }
        } else {
            std::path::PathBuf::from(&context.cwd)
        };

        let (command, args) = if let Some(args) = parsed.args {
            (parsed.command.trim().to_string(), args)
        } else {
            let parts = split_command_line(parsed.command.trim());
            if parts.is_empty() {
                return ToolResult::err("Command not allowed: empty command");
            }
            (parts[0].clone(), parts[1..].to_vec())
        };

        let use_shell = looks_like_shell_snippet(&parsed.command, &args);
        let background = is_background_shell_snippet(&parsed.command, &args);
        let known_command = is_allowed_command(&command);

        let exec = if use_shell {
            "bash".to_string()
        } else {
            command.clone()
        };
        let exec_args = if use_shell {
            let script = if background {
                parsed
                    .command
                    .trim()
                    .trim_end_matches('&')
                    .trim()
                    .to_string()
            } else {
                parsed.command.clone()
            };
            vec!["-lc".to_string(), script]
        } else {
            args.clone()
        };

        if let Some(perms) = &context.permissions {
            let approval = if !use_shell && !known_command {
                perms
                    .ensure_command(
                        &exec,
                        &exec_args,
                        effective_cwd.to_string_lossy().as_ref(),
                        Some(EnsureCommandOptions {
                            force_prompt_reason: Some(format!(
                                "Unknown command '{}' is not in the built-in read-only/development set",
                                command
                            )),
                        }),
                    )
                    .await
            } else if use_shell || !is_read_only_command(&command) {
                perms
                    .ensure_command(
                        &exec,
                        &exec_args,
                        effective_cwd.to_string_lossy().as_ref(),
                        None,
                    )
                    .await
            } else {
                Ok(())
            };

            if let Err(err) = approval {
                return ToolResult::err(err.to_string());
            }
        }

        if use_shell && background {
            let mut cmd = Command::new(&exec);
            cmd.args(&exec_args)
                .current_dir(&effective_cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            match cmd.spawn() {
                Ok(child) => {
                    let pid = child.id().unwrap_or_default() as i32;
                    let command_text = parsed
                        .command
                        .trim()
                        .trim_end_matches('&')
                        .trim()
                        .to_string();
                    let bg = register_background_shell_task(
                        &command_text,
                        pid,
                        effective_cwd.to_string_lossy().as_ref(),
                    );
                    ToolResult {
                        ok: true,
                        output: format!(
                            "Background command started.\nTASK: {}\nPID: {}",
                            bg.task_id, bg.pid
                        ),
                        background_task: Some(bg),
                        await_user: false,
                    }
                }
                Err(err) => ToolResult::err(err.to_string()),
            }
        } else {
            let out = Command::new(&exec)
                .args(&exec_args)
                .current_dir(&effective_cwd)
                .output()
                .await;
            match out {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    let output = [stdout, stderr]
                        .into_iter()
                        .filter(|x| !x.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult::ok(output)
                }
                Err(err) => ToolResult::err(err.to_string()),
            }
        }
    }
}

#[derive(Default)]
pub struct WebSearchTool;
#[derive(Debug, Deserialize)]
struct WebSearchInput {
    query: String,
    max_results: Option<usize>,
    allowed_domains: Option<Vec<String>>,
    blocked_domains: Option<Vec<String>>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web using DuckDuckGo Lite."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "query":{"type":"string","description":"Search query."},
                "max_results":{"type":"number","description":"Maximum number of results to return. Defaults to 5."},
                "allowed_domains":{"type":"array","items":{"type":"string"},"description":"Only return results from these domains."},
                "blocked_domains":{"type":"array","items":{"type":"string"},"description":"Exclude results from these domains."}
            },
            "required":["query"]
        })
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let parsed: WebSearchInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let query = parsed.query.trim();
        if query.is_empty() {
            return ToolResult::err("query is required");
        }

        match search_duckduckgo_lite(
            query,
            parsed.max_results.unwrap_or(5),
            parsed.allowed_domains.unwrap_or_default(),
            parsed.blocked_domains.unwrap_or_default(),
        )
        .await
        {
            Ok(items) => {
                if items.is_empty() {
                    return ToolResult::ok("No results found.");
                }
                let mut lines = vec![format!("QUERY: {query}"), String::new()];
                for (idx, item) in items.iter().enumerate() {
                    lines.push(format!("[{}] {}", idx + 1, item.title));
                    lines.push(format!("    URL: {}", item.link));
                    if !item.snippet.is_empty() {
                        lines.push(format!("    {}", item.snippet));
                    }
                    lines.push(String::new());
                }
                ToolResult::ok(lines.join("\n").trim_end().to_string())
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Default)]
pub struct WebFetchTool;
#[derive(Debug, Deserialize)]
struct WebFetchInput {
    url: String,
    max_chars: Option<usize>,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page and extract readable text content."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "url":{"type":"string","description":"HTTP or HTTPS URL to fetch."},
                "max_chars":{"type":"number","description":"Maximum number of characters to return. Defaults to 12000."}
            },
            "required":["url"]
        })
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let parsed: WebFetchInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let max_chars = parsed.max_chars.unwrap_or(12_000).max(500);
        if parsed.url.trim().is_empty() {
            return ToolResult::err("url is required");
        }

        match fetch_web_page(&parsed.url, max_chars).await {
            Ok(page) => {
                if page.status >= 400 {
                    return ToolResult::err(format!(
                        "HTTP {} {}: {}",
                        page.status, page.status_text, parsed.url
                    ));
                }
                let mut lines = vec![
                    format!("URL: {}", page.final_url),
                    format!("STATUS: {}", page.status),
                    format!("CONTENT_TYPE: {}", page.content_type),
                ];
                if let Some(title) = page.title
                    && !title.is_empty()
                {
                    lines.push(format!("TITLE: {}", title));
                }
                lines.push(String::new());
                lines.push(page.content);
                ToolResult::ok(lines.join("\n"))
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
struct WebSearchResult {
    title: String,
    link: String,
    snippet: String,
}

#[derive(Debug, Clone)]
struct FetchedPage {
    final_url: String,
    status: u16,
    status_text: String,
    content_type: String,
    title: Option<String>,
    content: String,
}

async fn search_duckduckgo_lite(
    query: &str,
    max_results: usize,
    allowed_domains: Vec<String>,
    blocked_domains: Vec<String>,
) -> Result<Vec<WebSearchResult>> {
    let client = reqwest::Client::builder()
        .user_agent(WEB_USER_AGENT)
        .build()?;

    let mut url = reqwest::Url::parse("https://lite.duckduckgo.com/lite/")?;
    url.query_pairs_mut().append_pair("q", query);

    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("Search request failed with status {}", response.status());
    }

    let html = response.text().await?;
    let allowed = normalize_domain_list(&allowed_domains);
    let blocked = normalize_domain_list(&blocked_domains);

    let mut parsed = parse_duckduckgo_lite(&html);
    parsed.retain(|r| passes_domain_filter(&r.link, &allowed, &blocked));
    parsed.truncate(max_results.clamp(1, 20));
    Ok(parsed)
}

async fn fetch_web_page(url: &str, max_chars: usize) -> Result<FetchedPage> {
    let client = reqwest::Client::builder()
        .user_agent(WEB_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8,*/*;q=0.7",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .send()
        .await?;

    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("Unknown")
        .to_string();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response.text().await?;

    if content_type.to_ascii_lowercase().contains("html") {
        Ok(FetchedPage {
            final_url,
            status,
            status_text,
            content_type,
            title: extract_title(&text),
            content: truncate_chars(&extract_readable_text(&text), max_chars),
        })
    } else {
        Ok(FetchedPage {
            final_url,
            status,
            status_text,
            content_type,
            title: None,
            content: truncate_chars(&text, max_chars),
        })
    }
}

fn parse_duckduckgo_lite(html: &str) -> Vec<WebSearchResult> {
    let mut results = vec![];
    let marker = "<a rel=\"nofollow\" href=\"";
    let mut cursor = 0usize;

    while let Some(link_pos_rel) = html[cursor..].find(marker) {
        let link_pos = cursor + link_pos_rel;
        let href_start = link_pos + marker.len();
        let Some(href_end_rel) = html[href_start..].find('"') else {
            break;
        };
        let href_end = href_start + href_end_rel;
        let raw_href = &html[href_start..href_end];

        let title_start_marker = "class='result-link'>";
        let Some(title_start_rel) = html[href_end..].find(title_start_marker) else {
            cursor = href_end;
            continue;
        };
        let title_start = href_end + title_start_rel + title_start_marker.len();
        let Some(title_end_rel) = html[title_start..].find("</a>") else {
            cursor = title_start;
            continue;
        };
        let title_end = title_start + title_end_rel;

        let next_anchor = html[title_end..]
            .find(marker)
            .map(|i| i + title_end)
            .unwrap_or(html.len());
        let block = &html[title_end..next_anchor];
        let snippet = extract_between(block, "<td class='result-snippet'>", "</td>")
            .map(|s| strip_tags(&s))
            .unwrap_or_default();

        let title = strip_tags(&decode_html(&html[title_start..title_end]));
        let link = normalize_duckduckgo_link(raw_href);
        if !title.is_empty() && !link.is_empty() {
            results.push(WebSearchResult {
                title,
                link,
                snippet: decode_html(&snippet),
            });
        }
        cursor = title_end;
    }

    results
}

fn normalize_domain_list(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

fn passes_domain_filter(link: &str, allowed: &[String], blocked: &[String]) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else {
        return false;
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if blocked.iter().any(|d| matches_domain(&host, d)) {
        return false;
    }
    if allowed.is_empty() {
        return true;
    }
    allowed.iter().any(|d| matches_domain(&host, d))
}

fn matches_domain(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn normalize_duckduckgo_link(raw_href: &str) -> String {
    let href = decode_html(raw_href).trim().to_string();
    if href.is_empty() {
        return String::new();
    }
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href
    };
    if let Ok(url) = reqwest::Url::parse(&absolute)
        && let Some(redirect) = url.query_pairs().find_map(|(k, v)| {
            if k == "uddg" {
                Some(v.into_owned())
            } else {
                None
            }
        })
    {
        return redirect;
    }
    absolute
}

fn extract_title(html: &str) -> Option<String> {
    extract_between(html, "<title", "</title>").map(|raw| {
        let title_text = raw
            .split_once('>')
            .map(|(_, right)| right)
            .unwrap_or(raw.as_str());
        strip_tags(&decode_html(title_text))
    })
}

fn extract_readable_text(html: &str) -> String {
    let mut text = html.to_string();
    for (start, end) in [
        ("<script", "</script>"),
        ("<style", "</style>"),
        ("<noscript", "</noscript>"),
        ("<svg", "</svg>"),
    ] {
        text = remove_block_like(&text, start, end);
    }
    text = strip_tags(&text);
    decode_html(&text)
}

fn remove_block_like(text: &str, start_tag: &str, end_tag: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start_pos) = rest.to_ascii_lowercase().find(&start_tag.to_ascii_lowercase()) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start_pos]);
        let after_start = &rest[start_pos..];
        let Some(end_pos_rel) = after_start
            .to_ascii_lowercase()
            .find(&end_tag.to_ascii_lowercase())
        else {
            break;
        };
        let end_pos = start_pos + end_pos_rel + end_tag.len();
        rest = &rest[end_pos..];
        out.push(' ');
    }
    out
}

fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/")
        .replace("&nbsp;", " ")
}

fn extract_between(text: &str, start: &str, end: &str) -> Option<String> {
    let start_idx = text.find(start)?;
    let right = &text[start_idx + start.len()..];
    let end_idx = right.find(end)?;
    Some(right[..end_idx].to_string())
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect::<String>()
}

/// 创建默认工具注册表，并按配置注入 MCP 工具。
pub async fn create_default_tool_registry(
    cwd: &std::path::Path,
    runtime: Option<&RuntimeConfig>,
) -> Result<ToolRegistry> {
    let skills = discover_skills(cwd);
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(AskUserTool),
        Arc::new(ListFilesTool),
        Arc::new(GrepFilesTool),
        Arc::new(ReadFileTool),
        Arc::new(WriteLikeTool {
            name: "write_file",
            description: "写入 UTF-8 文本文件。",
        }),
        Arc::new(WriteLikeTool {
            name: "modify_file",
            description: "替换文件全部内容（带 diff 审核）。",
        }),
        Arc::new(EditFileTool),
        Arc::new(PatchFileTool),
        Arc::new(RunCommandTool),
        Arc::new(WebSearchTool),
        Arc::new(WebFetchTool),
        Arc::new(LoadSkillTool::new(cwd.to_path_buf())),
    ];
    let mut mcp_server_summaries = vec![];
    let mut mcp_disposer = None;
    if let Some(runtime) = runtime {
        let mcp = create_mcp_backed_tools(cwd, &runtime.mcp_servers).await;
        tools.extend(mcp.tools);
        mcp_server_summaries = mcp.servers;
        mcp_disposer = mcp.disposer;
    }

    Ok(ToolRegistry::new(
        tools,
        skills,
        mcp_server_summaries,
        mcp_disposer,
    ))
}
