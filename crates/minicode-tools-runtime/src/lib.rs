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
