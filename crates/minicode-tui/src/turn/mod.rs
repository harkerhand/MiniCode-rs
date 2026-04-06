use std::io::Stdout;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self};
use minicode_agent_core::run_agent_turn;
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
mod busy_input;
mod event_apply;
mod prompt_handler;

pub(crate) use approval::handle_approval_key;
use busy_input::handle_busy_event;
use event_apply::{apply_turn_event, push_error_to_session};
use prompt_handler::build_prompt_handler;

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

    let _ = add_history_entry(&input);
    state.history = load_history_entries();
    state.history_index = state.history.len();
    state.history_draft.clear();

    match try_handle_local_command(&input).await {
        Ok(Some(local)) => {
            let messages = runtime_messages();
            state.context_tokens_estimate = estimate_context_tokens(&messages);
            append_runtime_message(ChatMessage::Assistant { content: local });
            state.transcript_scroll_offset = 0;
            state.history = load_history_entries();
            state.history_index = state.history.len();
            state.history_draft.clear();
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
        permission_manager.set_prompt_handler(build_prompt_handler(tx.clone()));
        let payload = shortcut.input;
        let tool_name_owned = shortcut.tool_name.to_string();

        tokio::spawn(async move {
            let _ = tx.send(TurnEvent::ToolStart {
                tool_name: tool_name_owned.clone(),
                input: payload.clone(),
            });
            let result = get_tool_registry().execute(&tool_name_owned, payload).await;
            let _ = tx.send(TurnEvent::ToolDone(result));
        });

        let mut tool_done = false;
        while state.is_busy {
            while let Ok(event) = rx.try_recv() {
                if matches!(event, TurnEvent::ToolDone(_)) {
                    tool_done = true;
                }
                let _ = apply_turn_event(state, event);
                if tool_done {
                    state.is_busy = false;
                }
            }
            render_screen(terminal, state)?;
            if event::poll(Duration::from_millis(60))? {
                let input_event = event::read()?;
                handle_busy_event(state, input_event);
            }
        }
        return Ok(false);
    }

    if input.starts_with('/') {
        let matches = find_matching_slash_commands(&input);
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
        append_runtime_message(ChatMessage::Assistant { content: msg });
        return Ok(false);
    }

    append_runtime_message(ChatMessage::User {
        content: input.clone(),
    });
    let messages = runtime_messages();
    state.context_tokens_estimate = estimate_context_tokens(&messages);

    permission_manager.begin_turn();
    state.status = Some("Thinking...".to_string());
    state.is_busy = true;

    let (tx, mut rx) = mpsc::unbounded_channel::<TurnEvent>();
    permission_manager.set_prompt_handler(build_prompt_handler(tx.clone()));
    let model = get_model_adapter();

    tokio::spawn(async move {
        let mut callbacks = ChannelCallbacks { tx: tx.clone() };
        run_agent_turn(model.as_ref(), None, Some(&mut callbacks)).await;
        let _ = tx.send(TurnEvent::Done);
    });

    let mut turn_done = false;
    while !turn_done {
        while let Ok(event) = rx.try_recv() {
            if apply_turn_event(state, event) {
                turn_done = true;
                break;
            }
        }

        render_screen(terminal, state)?;

        if !turn_done && event::poll(Duration::from_millis(60))? {
            let input_event = event::read()?;
            handle_busy_event(state, input_event);
        }
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
