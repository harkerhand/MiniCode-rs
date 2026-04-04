use std::io::Stdout;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use minicode_agent_core::run_agent_turn;
use minicode_cli_commands::{find_matching_slash_commands, try_handle_local_command};
use minicode_core::history::save_history_entries;
use minicode_core::prompt::build_system_prompt;
use minicode_core::types::ChatMessage;
use minicode_permissions::{
    PermissionDecision, PermissionPromptHandler, PermissionPromptResult, session_permissions,
};
use minicode_shortcuts::parse_local_tool_shortcut;
use minicode_tool::ToolContext;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::{mpsc, oneshot};

use crate::input::{scroll_transcript_by, toggle_tool_details};
use crate::render::render_screen;
use crate::state::{
    ChannelCallbacks, PendingApproval, ScreenState, TranscriptEntry, TuiAppArgs, TurnEvent,
};

/// 向会话转录中写入一条错误消息并更新状态。
fn push_error_to_session(state: &mut ScreenState, message: impl Into<String>) {
    state.transcript.push(TranscriptEntry {
        kind: "tool:error".to_string(),
        body: message.into(),
    });
    state.transcript_scroll_offset = 0;
    state.status = Some("Error".to_string());
}

/// 为工具输入生成便于展示的简短摘要。
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
        return format!("{} path={}", tool_name, path);
    }
    if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
        return format!("{} {}", tool_name, command);
    }
    serde_json::to_string(input).unwrap_or_else(|_| "(invalid input)".to_string())
}

/// 应用单个回合事件到 UI 状态，必要时返回新消息列表。
fn apply_turn_event(state: &mut ScreenState, event: TurnEvent) -> Option<Vec<ChatMessage>> {
    match event {
        TurnEvent::ToolStart { tool_name, input } => {
            state.active_tool = Some(tool_name.clone());
            state.status = Some(format!("Running {tool_name}..."));
            state.transcript.push(TranscriptEntry {
                kind: "tool".to_string(),
                body: format!(
                    "{}\n{}",
                    tool_name,
                    summarize_tool_input(&tool_name, &input)
                ),
            });
            state.transcript_scroll_offset = 0;
            None
        }
        TurnEvent::ToolResult {
            tool_name,
            output,
            is_error,
        } => {
            state.recent_tools.push((tool_name, !is_error));
            state.transcript.push(TranscriptEntry {
                kind: if is_error {
                    "tool:error".to_string()
                } else {
                    "tool".to_string()
                },
                body: output,
            });
            state.transcript_scroll_offset = 0;
            None
        }
        TurnEvent::Assistant(content) => {
            state.transcript.push(TranscriptEntry {
                kind: "assistant".to_string(),
                body: content,
            });
            state.transcript_scroll_offset = 0;
            None
        }
        TurnEvent::Progress(content) => {
            state.transcript.push(TranscriptEntry {
                kind: "progress".to_string(),
                body: content,
            });
            state.transcript_scroll_offset = 0;
            None
        }
        TurnEvent::Approval { request, responder } => {
            state.pending_approval = Some(PendingApproval {
                request,
                responder: Some(responder),
                selected_index: 0,
                awaiting_feedback: false,
                feedback: String::new(),
            });
            state.status = Some("Approval required...".to_string());
            None
        }
        TurnEvent::ToolDone(result) => {
            state.recent_tools.push((
                state
                    .active_tool
                    .clone()
                    .unwrap_or_else(|| "tool".to_string()),
                result.ok,
            ));
            state.transcript.push(TranscriptEntry {
                kind: if result.ok {
                    "tool".to_string()
                } else {
                    "tool:error".to_string()
                },
                body: result.output,
            });
            state.active_tool = None;
            state.status = None;
            None
        }
        TurnEvent::Done(updated) => Some(updated),
    }
}

/// 在模型忙碌期间处理允许的键鼠事件。
fn handle_busy_event(state: &mut ScreenState, event: Event) {
    match event {
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                let _ = scroll_transcript_by(state, 3);
            }
            MouseEventKind::ScrollDown => {
                let _ = scroll_transcript_by(state, -3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, entry_index)) = state
                    .visible_tool_toggle_rows
                    .iter()
                    .find(|(y, _)| *y == mouse.row)
                    .copied()
                {
                    let _ = toggle_tool_details(state, entry_index);
                }
            }
            _ => {}
        },
        Event::Key(key) => {
            if state.pending_approval.is_some() && handle_approval_key(state, key) {
                return;
            }

            match key {
                KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                } => {
                    let _ = scroll_transcript_by(state, 8);
                }
                KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                } => {
                    let _ = scroll_transcript_by(state, -8);
                }
                KeyEvent {
                    code: KeyCode::Up,
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::ALT) => {
                    let _ = scroll_transcript_by(state, 1);
                }
                KeyEvent {
                    code: KeyCode::Down,
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::ALT) => {
                    let _ = scroll_transcript_by(state, -1);
                }
                KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    state.transcript_scroll_offset = state.session_max_scroll_offset;
                }
                KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    state.transcript_scroll_offset = 0;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// 处理权限审批弹窗中的键盘交互。
pub(crate) fn handle_approval_key(state: &mut ScreenState, key: KeyEvent) -> bool {
    let Some(pending) = state.pending_approval.as_mut() else {
        return false;
    };

    let choices_len = pending.request.choices.len();
    if choices_len == 0 {
        return false;
    }

    let selected_decision = pending.request.choices[pending.selected_index].decision;

    if pending.awaiting_feedback {
        match key.code {
            KeyCode::Enter => {
                if let Some(tx) = pending.responder.take() {
                    let _ = tx.send(PermissionPromptResult {
                        decision: PermissionDecision::DenyWithFeedback,
                        feedback: Some(pending.feedback.clone()),
                    });
                }
                state.pending_approval = None;
                state.status = Some("Thinking...".to_string());
                return true;
            }
            KeyCode::Backspace => {
                pending.feedback.pop();
                return true;
            }
            KeyCode::Esc => {
                pending.awaiting_feedback = false;
                pending.feedback.clear();
                return true;
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    pending.feedback.push(ch);
                    return true;
                }
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Left | KeyCode::Up => {
            pending.selected_index = if pending.selected_index == 0 {
                choices_len - 1
            } else {
                pending.selected_index - 1
            };
            true
        }
        KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
            pending.selected_index = (pending.selected_index + 1) % choices_len;
            true
        }
        KeyCode::Char(ch) => {
            let lower = ch.to_ascii_lowercase().to_string();
            if let Some(idx) = pending
                .request
                .choices
                .iter()
                .position(|c| c.key.eq_ignore_ascii_case(&lower))
            {
                pending.selected_index = idx;
                return true;
            }
            false
        }
        KeyCode::Enter => {
            if selected_decision == PermissionDecision::DenyWithFeedback {
                pending.awaiting_feedback = true;
                return true;
            }
            if let Some(tx) = pending.responder.take() {
                let _ = tx.send(PermissionPromptResult {
                    decision: selected_decision,
                    feedback: None,
                });
            }
            state.pending_approval = None;
            state.status = Some("Thinking...".to_string());
            true
        }
        KeyCode::Esc => {
            if let Some(tx) = pending.responder.take() {
                let _ = tx.send(PermissionPromptResult {
                    decision: PermissionDecision::DenyOnce,
                    feedback: None,
                });
            }
            state.pending_approval = None;
            state.status = Some("Thinking...".to_string());
            true
        }
        _ => false,
    }
}

/// 构造将权限请求转发到 UI 的回调处理器。
fn build_prompt_handler(tx: mpsc::UnboundedSender<TurnEvent>) -> PermissionPromptHandler {
    Arc::new(move |request| {
        let event_tx = tx.clone();
        Box::pin(async move {
            let (decision_tx, decision_rx) = oneshot::channel();
            if event_tx
                .send(TurnEvent::Approval {
                    request,
                    responder: decision_tx,
                })
                .is_err()
            {
                return PermissionPromptResult {
                    decision: PermissionDecision::DenyOnce,
                    feedback: None,
                };
            }
            match decision_rx.await {
                Ok(v) => v,
                Err(_) => PermissionPromptResult {
                    decision: PermissionDecision::DenyOnce,
                    feedback: None,
                },
            }
        })
    })
}

/// 处理用户提交：本地命令、快捷工具或模型回合。
pub(crate) async fn handle_submit(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    args: &mut TuiAppArgs,
    state: &mut ScreenState,
    messages: &mut Vec<ChatMessage>,
    raw_input: String,
) -> Result<bool> {
    let permissions = session_permissions();
    let input = raw_input.trim().to_string();
    if input.is_empty() {
        return Ok(false);
    }
    if input == "/exit" {
        return Ok(true);
    }

    if state.history.last().map(|x| x.as_str()) != Some(input.as_str()) {
        state.history.push(input.clone());
        let _ = save_history_entries(&state.history);
    }
    state.history_index = state.history.len();
    state.history_draft.clear();

    if input == "/tools" {
        state.transcript.push(TranscriptEntry {
            kind: "assistant".to_string(),
            body: args
                .tools
                .list()
                .iter()
                .map(|tool| format!("{}: {}", tool.name(), tool.description()))
                .collect::<Vec<_>>()
                .join("\n"),
        });
        return Ok(false);
    }

    match try_handle_local_command(&input, &args.cwd, Some(&args.tools)).await {
        Ok(Some(local)) => {
            state.transcript.push(TranscriptEntry {
                kind: "assistant".to_string(),
                body: local,
            });
            return Ok(false);
        }
        Ok(None) => {}
        Err(err) => {
            push_error_to_session(state, format!("local command failed: {err:#}"));
            return Ok(false);
        }
    }

    if let Some(shortcut) = parse_local_tool_shortcut(&input) {
        state.is_busy = true;
        state.status = Some(format!("Running {}...", shortcut.tool_name));
        let (tx, mut rx) = mpsc::unbounded_channel::<TurnEvent>();
        let task_permissions = permissions.clone();
        task_permissions.set_prompt_handler(build_prompt_handler(tx.clone()));
        let tools = args.tools.clone();
        let cwd = args.cwd.to_string_lossy().to_string();
        let payload = shortcut.input;
        let tool_name_owned = shortcut.tool_name.to_string();

        tokio::spawn(async move {
            let _ = tx.send(TurnEvent::ToolStart {
                tool_name: tool_name_owned.clone(),
                input: payload.clone(),
            });
            let result = tools
                .execute(
                    &tool_name_owned,
                    payload,
                    &ToolContext {
                        cwd,
                        permissions: Some(Arc::new(task_permissions)),
                    },
                )
                .await;
            let _ = tx.send(TurnEvent::ToolDone(result));
        });

        while state.is_busy {
            while let Ok(event) = rx.try_recv() {
                let _ = apply_turn_event(state, event);
                if state.pending_approval.is_none() {
                    state.is_busy = false;
                }
            }
            render_screen(terminal, args, state)?;
            if event::poll(Duration::from_millis(60))? {
                let input_event = event::read()?;
                handle_busy_event(state, input_event);
            }
        }
        return Ok(false);
    }

    if input.starts_with('/') {
        let matches = find_matching_slash_commands(&input);
        state.transcript.push(TranscriptEntry {
            kind: "assistant".to_string(),
            body: if matches.is_empty() {
                "未识别命令。输入 /help 查看可用命令。".to_string()
            } else {
                format!("未识别命令。你是不是想输入：\n{}", matches.join("\n"))
            },
        });
        return Ok(false);
    }

    let skills = args.tools.get_skills();
    let mcp_servers = args.tools.get_mcp_servers();
    messages[0] = ChatMessage::System {
        content: build_system_prompt(&args.cwd, &permissions.get_summary(), &skills, &mcp_servers),
    };
    messages.push(ChatMessage::User {
        content: input.clone(),
    });
    state.transcript.push(TranscriptEntry {
        kind: "user".to_string(),
        body: input,
    });

    permissions.begin_turn();
    state.status = Some("Thinking...".to_string());
    state.is_busy = true;

    let (tx, mut rx) = mpsc::unbounded_channel::<TurnEvent>();
    let task_permissions = permissions.clone();
    task_permissions.set_prompt_handler(build_prompt_handler(tx.clone()));
    let tools = args.tools.clone();
    let model = args.model.clone();
    let current_messages = messages.clone();
    let cwd = args.cwd.to_string_lossy().to_string();

    tokio::spawn(async move {
        let mut callbacks = ChannelCallbacks { tx: tx.clone() };
        let updated = run_agent_turn(
            model.as_ref(),
            &tools,
            current_messages,
            ToolContext {
                cwd,
                permissions: Some(Arc::new(task_permissions)),
            },
            None,
            Some(&mut callbacks),
        )
        .await;
        let _ = tx.send(TurnEvent::Done(updated));
    });

    let mut done_messages: Option<Vec<ChatMessage>> = None;
    while done_messages.is_none() {
        while let Ok(event) = rx.try_recv() {
            if let Some(done) = apply_turn_event(state, event) {
                done_messages = Some(done);
                break;
            }
        }

        render_screen(terminal, args, state)?;

        if done_messages.is_none() && event::poll(Duration::from_millis(60))? {
            let input_event = event::read()?;
            handle_busy_event(state, input_event);
        }
    }

    *messages = done_messages.unwrap_or_default();
    permissions.end_turn();
    state.is_busy = false;
    state.status = None;
    state.active_tool = None;
    state.pending_approval = None;
    Ok(false)
}
