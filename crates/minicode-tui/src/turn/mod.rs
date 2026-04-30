use std::io::Stdout;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self};
use minicode_agent_core::run_agent_turn_streaming;
use minicode_cli_commands::{find_matching_slash_commands, try_handle_local_command};
use minicode_history::{
    add_history_entry, append_runtime_message, estimate_context_tokens, load_history_entries,
    runtime_messages,
};
use minicode_permissions::get_permission_manager;
use minicode_tool::{get_tool_registry, parse_local_tool_shortcut};
use minicode_types::{ChatMessage, get_model_adapter};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::render::render_screen;
use crate::state::{ChannelCallbacks, ScreenState, TurnEvent};

mod approval;
mod ask_user;
mod busy_input;
mod event_apply;
mod prompt_handler;

pub(crate) use approval::handle_approval_key;
pub(crate) use ask_user::{AskUserAction, handle_ask_user_key};
use busy_input::{BusyEventAction, handle_busy_event};
use event_apply::apply_turn_event;
use prompt_handler::build_prompt_handler;

const UI_POLL_MS: u64 = 16;

async fn handle_command_submission(state: &mut ScreenState, input: &str) {
    append_runtime_message(ChatMessage::runtime_display(
        "command",
        format!("> {input}"),
    ));
    match try_handle_local_command(input).await {
        Ok(Some(local)) => {
            append_runtime_message(ChatMessage::runtime_display("command:result", local));
        }
        Ok(None) => {
            let matches = find_matching_slash_commands(input);
            let msg = if matches.is_empty() {
                "未识别命令。输入 /help 查看可用命令。".to_string()
            } else {
                format!(
                    "未识别命令。你是不是想输入：\n{}",
                    matches
                        .iter()
                        .map(|(usage, _)| usage.clone())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            append_runtime_message(ChatMessage::runtime_display("command:error", msg));
        }
        Err(err) => {
            append_runtime_message(ChatMessage::runtime_display(
                "command:error",
                format!("local command failed: {err:#}"),
            ));
        }
    }
    state.transcript_scroll_offset = 0;
}

async fn queue_busy_submission(state: &mut ScreenState, raw: String) {
    let input = raw.trim().to_string();
    if input.is_empty() {
        return;
    }
    if input.starts_with('/') {
        handle_command_submission(state, &input).await;
        return;
    }
    let _ = add_history_entry(&input);
    state.history = load_history_entries();
    state.history_index = state.history.len();
    state.history_draft.clear();
    state.queued_busy_inputs.push(input);
    state.status = Some("新输入等待注入上下文...".to_string());
}

fn flush_queued_busy_inputs(state: &mut ScreenState) {
    if state.queued_busy_inputs.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut state.queued_busy_inputs);
    for content in pending {
        append_runtime_message(ChatMessage::User { content });
    }
    state.context_tokens_estimate = estimate_context_tokens(&runtime_messages());
    state.transcript_scroll_offset = 0;
    if let Some(tool) = state.active_tool.as_ref() {
        state.status = Some(format!("Running {tool}..."));
    }
}

/// 处理用户提交：本地命令、快捷工具或模型回合。
pub(crate) async fn handle_submit(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut ScreenState,
    raw_input: String,
) -> Result<bool> {
    let permission_manager = get_permission_manager();
    let input = raw_input.trim().to_string();
    if input.is_empty() {
        return Ok(false);
    }
    if input == "/exit" {
        return Ok(true);
    }

    if input.starts_with('/') {
        handle_command_submission(state, &input).await;
        return Ok(false);
    }

    if let Some(shortcut) = parse_local_tool_shortcut(&input) {
        append_runtime_message(ChatMessage::runtime_display(
            "command",
            format!("> {input}"),
        ));
        state.is_busy = true;
        state.status = Some(format!("Running {}...", shortcut.tool_name));
        let (tx, mut rx) = mpsc::unbounded_channel::<TurnEvent>();
        permission_manager
            .set_prompt_handler(build_prompt_handler(tx.clone()))
            .await;
        let payload = shortcut.input;
        let tool_name_owned = shortcut.tool_name.to_string();

        let task = tokio::spawn(async move {
            let _ = tx.send(TurnEvent::ToolStart {
                tool_name: tool_name_owned.clone(),
                input: payload.clone(),
            });
            let result = get_tool_registry().execute(&tool_name_owned, payload).await;
            let _ = tx.send(TurnEvent::ToolDone(result));
        });

        let mut tool_done = false;
        while state.is_busy {
            let mut updated = false;
            while let Ok(event) = rx.try_recv() {
                if matches!(event, TurnEvent::ToolDone(_)) {
                    tool_done = true;
                }
                let _ = apply_turn_event(state, event);
                updated = true;
                if tool_done {
                    flush_queued_busy_inputs(state);
                    state.is_busy = false;
                }
            }
            if updated {
                render_screen(terminal, state)?;
            }
            if event::poll(Duration::from_millis(UI_POLL_MS))? {
                let input_event = event::read()?;
                match handle_busy_event(state, input_event) {
                    BusyEventAction::None => {}
                    BusyEventAction::Submit(raw) => queue_busy_submission(state, raw).await,
                    BusyEventAction::Interrupt => {
                        task.abort();
                        append_runtime_message(ChatMessage::runtime_display(
                            "command:error",
                            "已中断当前轮次。",
                        ));
                        state.transcript_scroll_offset = 0;
                        state.is_busy = false;
                    }
                }
                render_screen(terminal, state)?;
            }
        }
        flush_queued_busy_inputs(state);
        return Ok(false);
    }

    let _ = add_history_entry(&input);
    state.history = load_history_entries();
    state.history_index = state.history.len();
    state.history_draft.clear();

    append_runtime_message(ChatMessage::User {
        content: input.clone(),
    });
    let messages = runtime_messages();
    state.context_tokens_estimate = estimate_context_tokens(&messages);

    permission_manager.begin_turn();
    state.status = Some("Thinking...".to_string());
    state.is_busy = true;
    state.stream_text.clear();
    state.stream_frozen = false;

    let (tx, mut rx) = mpsc::unbounded_channel::<TurnEvent>();
    permission_manager
        .set_prompt_handler(build_prompt_handler(tx.clone()))
        .await;
    let model = get_model_adapter();

    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<(String, bool)>();
    let forward_tx = tx.clone();
    tokio::spawn(async move {
        while let Some((delta, is_final)) = stream_rx.recv().await {
            let _ = forward_tx.send(TurnEvent::StreamDelta(delta, is_final));
        }
    });
    let mut task = tokio::spawn(async move {
        let mut callbacks = ChannelCallbacks { tx: tx.clone() };
        run_agent_turn_streaming(model.as_ref(), None, Some(&mut callbacks), Some(stream_tx))
            .await;
        let _ = tx.send(TurnEvent::Done);
    });

    // 循环处理：如果回合结束后有排队输入，自动发起新回合
    loop {
        let mut turn_done = false;
        while !turn_done {
            let mut updated = false;
            while let Ok(event) = rx.try_recv() {
                if matches!(event, TurnEvent::ToolResult { .. }) {
                    flush_queued_busy_inputs(state);
                }
                if apply_turn_event(state, event) {
                    turn_done = true;
                    break;
                }
                updated = true;
            }

            if updated {
                render_screen(terminal, state)?;
            }

            if !turn_done && event::poll(Duration::from_millis(UI_POLL_MS))? {
                let input_event = event::read()?;
                match handle_busy_event(state, input_event) {
                    BusyEventAction::None => {}
                    BusyEventAction::Submit(raw) => queue_busy_submission(state, raw).await,
                    BusyEventAction::Interrupt => {
                        task.abort();
                        append_runtime_message(ChatMessage::runtime_display(
                            "command:error",
                            "已中断当前轮次。",
                        ));
                        state.transcript_scroll_offset = 0;
                        turn_done = true;
                    }
                }
                render_screen(terminal, state)?;
            }
        }
        flush_queued_busy_inputs(state);

        // 回合结束后，若没有排队的新输入则退出循环
        if state.queued_busy_inputs.is_empty() {
            break;
        }
        // 有排队输入，自动发起新回合
        let (new_tx, new_rx) = mpsc::unbounded_channel::<TurnEvent>();
        let (new_stream_tx, new_stream_rx) = mpsc::unbounded_channel::<(String, bool)>();
        let new_forward_tx = new_tx.clone();
        tokio::spawn(async move {
            let mut new_stream_rx = new_stream_rx;
            while let Some((delta, is_final)) = new_stream_rx.recv().await {
                let _ = new_forward_tx.send(TurnEvent::StreamDelta(delta, is_final));
            }
        });
        permission_manager
            .set_prompt_handler(build_prompt_handler(new_tx.clone()))
            .await;
        let new_model = get_model_adapter();
        let new_task = tokio::spawn(async move {
            let mut callbacks = ChannelCallbacks { tx: new_tx.clone() };
            run_agent_turn_streaming(
                new_model.as_ref(),
                None,
                Some(&mut callbacks),
                Some(new_stream_tx),
            )
            .await;
            let _ = new_tx.send(TurnEvent::Done);
        });
        task = new_task;
        rx = new_rx;
        state.status = Some("Thinking...".to_string());
        state.stream_text.clear();
        state.stream_frozen = false;
    }

    let done = runtime_messages();
    state.context_tokens_estimate = estimate_context_tokens(&done);
    permission_manager.end_turn();
    state.is_busy = false;
    state.status = None;
    state.active_tool = None;
    state.pending_approval = None;
    Ok(false)
}
