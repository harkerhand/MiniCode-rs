use std::collections::HashSet;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use uuid::Uuid;

use crate::config::RuntimeConfig;
use crate::file_review::apply_reviewed_file_change;
use crate::mcp::{create_mcp_backed_tools, extend_registry_with_mcp};
use crate::skills::{discover_skills, load_skill};
use crate::tool::{BackgroundTaskResult, Tool, ToolContext, ToolRegistry, ToolResult};
use crate::workspace::resolve_tool_path;

#[derive(Default)]
pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }
    fn description(&self) -> &str {
        "向用户提问并暂停当前轮次。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"question":{"type":"string"}},"required":["question"]})
    }
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
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "列出目录内容（最多200条）。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
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
    fn name(&self) -> &str {
        "grep_files"
    }
    fn description(&self) -> &str {
        "使用 ripgrep 搜索文本。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"]})
    }
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
                ToolResult::ok(text)
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Default)]
pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "读取 UTF-8 文本文件，可按 offset/limit 分块。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["path"]})
    }
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
        let end = (offset + limit).min(content.len());
        let chunk = content.get(offset..end).unwrap_or("");
        let truncated = end < content.len();
        let header = format!(
            "FILE: {}\nOFFSET: {}\nEND: {}\nTOTAL_CHARS: {}\nTRUNCATED: {}\n\n",
            path,
            offset,
            end,
            content.len(),
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
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        self.description
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
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
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "通过精确 search/replace 修改文件。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"search":{"type":"string"},"replace":{"type":"string"},"replaceAll":{"type":"boolean"}},"required":["path","search","replace"]})
    }
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
}
#[async_trait]
impl Tool for PatchFileTool {
    fn name(&self) -> &str {
        "patch_file"
    }
    fn description(&self) -> &str {
        "对单文件执行批量替换。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"replacements":{"type":"array","items":{"type":"object","properties":{"search":{"type":"string"},"replace":{"type":"string"}},"required":["search","replace"]}}},"required":["path","replacements"]})
    }
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
            content = content.replacen(&rep.search, &rep.replace, 1);
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
    pub fn new(cwd: std::path::PathBuf) -> Self {
        Self { cwd }
    }
}
#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &str {
        "load_skill"
    }
    fn description(&self) -> &str {
        "读取某个技能的 SKILL.md 内容。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})
    }
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

fn split_command_line(command_line: &str) -> Vec<String> {
    shell_words::split(command_line).unwrap_or_else(|_| {
        command_line
            .split_whitespace()
            .map(str::to_string)
            .collect()
    })
}

fn looks_like_shell_snippet(command: &str, args: &[String]) -> bool {
    if !args.is_empty() {
        return false;
    }
    command.chars().any(|c| "|&;<>()$`".contains(c))
}

fn is_background_shell_snippet(command: &str, args: &[String]) -> bool {
    if !args.is_empty() {
        return false;
    }
    let t = command.trim();
    t.ends_with('&') && !t.ends_with("&&")
}

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }
    fn description(&self) -> &str {
        "运行常见开发命令。支持通过 command 传入完整 shell 片段。"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"}},"required":["command"]})
    }
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

        let allowlist: HashSet<&str> = [
            "pwd", "ls", "find", "rg", "cat", "echo", "env", "grep", "git", "npm", "node",
            "python3", "pytest", "bash", "sh", "bun", "sed", "head", "tail", "wc",
        ]
        .into_iter()
        .collect();

        let use_shell = looks_like_shell_snippet(&parsed.command, &args);
        let background = is_background_shell_snippet(&parsed.command, &args);

        if !use_shell && !allowlist.contains(command.as_str()) {
            return ToolResult::err(format!("Command not allowed: {}", command));
        }

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

        if let Some(perms) = &context.permissions
            && let Err(err) = perms
                .ensure_command(&exec, &exec_args, effective_cwd.to_string_lossy().as_ref())
                .await
        {
            return ToolResult::err(err.to_string());
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
                    let started = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or_default();
                    let bg = BackgroundTaskResult {
                        task_id: Uuid::new_v4().to_string(),
                        r#type: "local_bash".to_string(),
                        command: parsed
                            .command
                            .trim()
                            .trim_end_matches('&')
                            .trim()
                            .to_string(),
                        pid,
                        status: "running".to_string(),
                        started_at: started,
                    };
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

pub async fn create_default_tool_registry(
    cwd: &std::path::Path,
    runtime: Option<&RuntimeConfig>,
) -> Result<ToolRegistry> {
    let skills = discover_skills(cwd);
    let mcp = create_mcp_backed_tools(
        cwd,
        &runtime.map(|r| r.mcp_servers.clone()).unwrap_or_default(),
    )
    .await;

    let tools: Vec<Arc<dyn Tool>> = vec![
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

    Ok(extend_registry_with_mcp(tools, skills, mcp))
}
