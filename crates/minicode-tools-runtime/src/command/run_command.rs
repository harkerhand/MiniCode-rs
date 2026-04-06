use std::process::Stdio;

use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_background_tasks::register_background_shell_task;
use minicode_config::runtime_store;
use minicode_permissions::EnsureCommandOptions;
use minicode_permissions::get_permission_manager;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::process::Command;

#[derive(Debug, Deserialize)]

struct RunCommandInput {
    command: String,
    args: Option<Vec<String>>,
    cwd: Option<String>,
}

struct NormalizedCommandInput {
    command: String,
    args: Vec<String>,
}

#[derive(Default)]
pub struct RunCommandTool;
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
    async fn run(&self, input: Value) -> ToolResult {
        let parsed: RunCommandInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let effective_cwd = if let Some(cwd) = parsed.cwd.as_deref() {
            match resolve_tool_path(cwd, "list").await {
                Ok(v) => v,
                Err(err) => return ToolResult::err(err.to_string()),
            }
        } else {
            runtime_store().cwd.clone()
        };

        let normalized = normalize_command_input(&parsed);
        if normalized.command.is_empty() {
            return ToolResult::err("Command not allowed: empty command");
        }

        let use_shell = looks_like_shell_snippet(&parsed.command, parsed.args.as_ref());
        let background = is_background_shell_snippet(&parsed.command, parsed.args.as_ref());
        let known_command = is_allowed_command(&normalized.command);

        let exec = if use_shell {
            "bash".to_string()
        } else {
            normalized.command.clone()
        };
        let exec_args = if use_shell {
            let script = if background {
                strip_trailing_background_operator(&parsed.command)
            } else {
                parsed.command.clone()
            };
            vec!["-lc".to_string(), script]
        } else {
            normalized.args.clone()
        };

        let permission_manager = get_permission_manager();
        let approval = if !use_shell && !known_command {
            permission_manager
                .ensure_command(
                    &exec,
                    &exec_args,
                    effective_cwd.to_string_lossy().as_ref(),
                    Some(EnsureCommandOptions {
                        force_prompt_reason: Some(format!(
                            "Unknown command '{}' is not in the built-in read-only/development set",
                            normalized.command
                        )),
                    }),
                )
                .await
        } else if use_shell || !is_read_only_command(&normalized.command) {
            permission_manager
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

/// 归一化命令输入：优先使用显式 args；否则把单字符串拆分为 command + args。
fn normalize_command_input(input: &RunCommandInput) -> NormalizedCommandInput {
    if input.args.as_ref().is_some_and(|args| !args.is_empty()) {
        return NormalizedCommandInput {
            command: input.command.trim().to_string(),
            args: input.args.clone().unwrap_or_default(),
        };
    }

    let trimmed = input.command.trim();
    if trimmed.is_empty() {
        return NormalizedCommandInput {
            command: String::new(),
            args: Vec::new(),
        };
    }
    let parts = split_command_line(trimmed);
    let command = parts.first().cloned().unwrap_or_default();
    let args = if parts.len() > 1 {
        parts[1..].to_vec()
    } else {
        Vec::new()
    };
    NormalizedCommandInput { command, args }
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
fn looks_like_shell_snippet(command: &str, args: Option<&Vec<String>>) -> bool {
    if args.is_some_and(|items| !items.is_empty()) {
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
fn is_background_shell_snippet(command: &str, args: Option<&Vec<String>>) -> bool {
    if args.is_some_and(|items| !items.is_empty()) {
        return false;
    }
    let t = command.trim();
    t.ends_with('&') && !t.ends_with("&&")
}

/// 移除 shell 命令末尾用于后台运行的单个 `&`。
fn strip_trailing_background_operator(command: &str) -> String {
    command.trim().trim_end_matches('&').trim().to_string()
}
