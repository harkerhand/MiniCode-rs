use std::io::Stdout;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self};
use minicode_agent_core::run_agent_turn;
use minicode_cli_commands::{find_matching_slash_commands, try_handle_local_command};
use minicode_history::{
    add_history_entry, estimate_context_tokens, load_history_entries, runtime_messages,
    runtime_transcript,
    set_runtime_messages, set_runtime_transcript,
};
use minicode_permissions::session_permissions;
use minicode_prompt::build_system_prompt;
use minicode_tool::{ToolContext, parse_local_tool_shortcut};
use minicode_types::{ChatMessage, TranscriptLine};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::render::render_screen;
use crate::state::{ChannelCallbacks, ScreenState, TuiAppArgs, TurnEvent};

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
    args: &mut TuiAppArgs,
    state: &mut ScreenState,
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

    let history_entry = ChatMessage::User {
        content: input.clone(),
    };
    let _ = add_history_entry(&history_entry);
    state.history = load_history_entries();
    state.history_index = state.history.len();
    state.history_draft.clear();

    match try_handle_local_command(&input, &args.cwd, &args.tools).await {
        Ok(Some(local)) => {
            let messages = runtime_messages();
            state.context_tokens_estimate = estimate_context_tokens(&messages);
            state.transcript = runtime_transcript();
            state.transcript.push(TranscriptLine {
                kind: "assistant".to_string(),
                body: local,
            });
            state.transcript_scroll_offset = 0;
            set_runtime_transcript(state.transcript.clone());
            state.history = load_history_entries();
            state.history_index = state.history.len();
            state.history_draft.clear();
            return Ok(false);
        }
        Ok(None) => {}
        Err(err) => {
            push_error_to_session(state, format!("local command failed: {err:#}"));
            set_runtime_transcript(state.transcript.clone());
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

        let mut tool_done = false;
        while state.is_busy {
            while let Ok(event) = rx.try_recv() {
                if matches!(event, TurnEvent::ToolDone(_)) {
                    tool_done = true;
                }
                let _ = apply_turn_event(state, event);
                set_runtime_transcript(state.transcript.clone());
                if tool_done {
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
        state.transcript.push(TranscriptLine {
            kind: "assistant".to_string(),
            body: if matches.is_empty() {
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
            },
        });
        set_runtime_transcript(state.transcript.clone());
        return Ok(false);
    }

    let skills = args.tools.get_skills();
    let mcp_servers = args.tools.get_mcp_servers();

    let history_messages = runtime_messages();
    let mut next_messages = history_messages.clone();
    next_messages.push(ChatMessage::User {
        content: input.clone(),
    });

    let mut current_messages = Vec::with_capacity(next_messages.len() + 1);
    current_messages.push(ChatMessage::System {
        content: build_system_prompt(
            &args.cwd,
            &permissions.get_summary_text(),
            &skills,
            &mcp_servers,
        ),
    });
    current_messages.extend(next_messages.clone());

    state.context_tokens_estimate = estimate_context_tokens(&current_messages);
    state.transcript.push(TranscriptLine {
        kind: "user".to_string(),
        body: input,
    });
    set_runtime_messages(next_messages);
    set_runtime_transcript(state.transcript.clone());

    permissions.begin_turn();
    state.status = Some("Thinking...".to_string());
    state.is_busy = true;

    let (tx, mut rx) = mpsc::unbounded_channel::<TurnEvent>();
    let task_permissions = permissions.clone();
    task_permissions.set_prompt_handler(build_prompt_handler(tx.clone()));
    let tools = args.tools.clone();
    let model = args.model.clone();
    let current_messages = current_messages;
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
        set_runtime_messages(updated);
        let _ = tx.send(TurnEvent::Done);
    });

    let mut turn_done = false;
    while !turn_done {
        while let Ok(event) = rx.try_recv() {
            if apply_turn_event(state, event) {
                turn_done = true;
                set_runtime_transcript(state.transcript.clone());
                break;
            }
            set_runtime_transcript(state.transcript.clone());
        }

        render_screen(terminal, args, state)?;

        if !turn_done && event::poll(Duration::from_millis(60))? {
            let input_event = event::read()?;
            handle_busy_event(state, input_event);
        }
    }

    let done = runtime_messages();
    set_runtime_messages(done.clone());
    state.context_tokens_estimate = estimate_context_tokens(&done);
    permissions.end_turn();
    state.is_busy = false;
    state.status = None;
    state.active_tool = None;
    state.pending_approval = None;
    set_runtime_transcript(state.transcript.clone());
    Ok(false)
}
