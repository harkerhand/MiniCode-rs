use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_background_tasks::register_background_shell_task;
use minicode_config::runtime_store;
use minicode_permissions::get_permission_manager;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

const DEFAULT_FOREGROUND_COMMAND_TIMEOUT_SECS: u64 = 120;
const PIPE_DRAIN_TIMEOUT_MILLIS: u64 = 300;

#[derive(Debug, Deserialize)]

struct RunCommandInput {
    command: String,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    timeout_secs: Option<u64>,
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
        json!({"type":"object","properties":{"command":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"},"timeout_secs":{"type":"integer","minimum":1}},"required":["command"]})
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
        let approval = if !is_read_only_command(&normalized.command) {
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
            run_foreground_command(&exec, &exec_args, &effective_cwd, parsed.timeout_secs).await
        }
    }
}

async fn run_foreground_command(
    exec: &str,
    exec_args: &[String],
    cwd: &std::path::Path,
    timeout_secs_input: Option<u64>,
) -> ToolResult {
    let timeout_secs = timeout_secs_input
        .filter(|v| *v > 0)
        .or_else(|| {
            std::env::var("MINICODE_RUN_COMMAND_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|v| *v > 0)
        })
        .unwrap_or(DEFAULT_FOREGROUND_COMMAND_TIMEOUT_SECS);

    let mut cmd = Command::new(exec);
    cmd.args(exec_args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return ToolResult::err(err.to_string()),
    };

    let Some(mut stdout) = child.stdout.take() else {
        return ToolResult::err("run_command failed: stdout pipe unavailable");
    };
    let Some(mut stderr) = child.stderr.take() else {
        return ToolResult::err("run_command failed: stderr pipe unavailable");
    };

    let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stdout_buf_task = Arc::clone(&stdout_buf);
    let stderr_buf_task = Arc::clone(&stderr_buf);

    let stdout_task = tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut out = stdout_buf_task.lock().await;
                    out.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        loop {
            match stderr.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut out = stderr_buf_task.lock().await;
                    out.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
    });

    let (timed_out, status) = match timeout(Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(wait_result) => match wait_result {
            Ok(status) => (false, Some(status)),
            Err(err) => return ToolResult::err(err.to_string()),
        },
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            (true, None)
        }
    };

    wait_pipe_reader(stdout_task).await;
    wait_pipe_reader(stderr_task).await;
    let stdout = String::from_utf8_lossy(&stdout_buf.lock().await)
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(&stderr_buf.lock().await)
        .trim()
        .to_string();
    let combined = [stdout, stderr]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if timed_out {
        if combined.is_empty() {
            return ToolResult::err(format!(
                "Command timed out after {}s: {}",
                timeout_secs,
                format_command_line(exec, exec_args)
            ));
        }
        return ToolResult::err(format!(
            "Command timed out after {}s: {}\nPartial output:\n{}",
            timeout_secs,
            format_command_line(exec, exec_args),
            combined
        ));
    }

    let Some(status) = status else {
        return ToolResult::err("run_command failed: missing exit status");
    };

    if status.success() {
        return ToolResult::ok(combined);
    }

    let code = status
        .code()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    if combined.is_empty() {
        ToolResult::err(format!("Command exited with status: {code}"))
    } else {
        ToolResult::err(format!("Command exited with status: {code}\n{combined}"))
    }
}

async fn wait_pipe_reader(mut handle: tokio::task::JoinHandle<()>) {
    tokio::select! {
        _ = &mut handle => {},
        _ = tokio::time::sleep(Duration::from_millis(PIPE_DRAIN_TIMEOUT_MILLIS)) => {
            handle.abort();
        }
    }
}

fn format_command_line(exec: &str, args: &[String]) -> String {
    if args.is_empty() {
        exec.to_string()
    } else {
        format!("{} {}", exec, args.join(" "))
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
