use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::Show;
use crossterm::event::{
    self, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use minicode_config::{runtime_config, runtime_store};
use minicode_history::{
    append_runtime_message, estimate_context_tokens, get_input_history, runtime_messages,
    runtime_messages_count,
};
use minicode_types::ChatMessage;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod input;
mod render;
mod state;
mod theme;
mod turn;

use input::{
    char_len, get_visible_commands, history_down, history_up, insert_char_at, remove_char_at,
    remove_char_before, scroll_transcript_by, toggle_tool_details,
};
use render::render_screen;
use state::ScreenState;
use turn::{handle_approval_key, handle_submit};

struct TerminalGuard;

impl TerminalGuard {
    /// 进入 TUI 模式并打开备用屏幕。
    fn enter() -> Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            Show
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    /// 退出时恢复终端状态。
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};

/// 运行主 TUI 事件循环并处理用户输入。
pub async fn run_tui_app() -> Result<()> {
    let _terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // 使用预先准备好的数据（在 run() 函数中已经加载并处理过）
    let initial_messages = runtime_messages();
    let history = get_input_history();
    let history = history.lock().map(|g| g.clone()).unwrap_or_default();
    let mut state = ScreenState {
        history,
        message_count: initial_messages.len(),
        context_tokens_estimate: estimate_context_tokens(&initial_messages),
        turn_count: 0,
        ..ScreenState::default()
    };
    state.history_index = state.history.len();

    let mut should_exit = false;
    while !should_exit {
        // 更新上下文 token 估计
        state.context_tokens_estimate = estimate_context_tokens(&runtime_messages());
        render_screen(&mut terminal, &mut state)?;

        if event::poll(Duration::from_millis(150))? {
            match event::read()? {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let _ = scroll_transcript_by(&mut state, 3);
                    }
                    MouseEventKind::ScrollDown => {
                        let _ = scroll_transcript_by(&mut state, -3);
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some((_, entry_index)) = state
                            .visible_tool_toggle_rows
                            .iter()
                            .find(|(y, _)| *y == mouse.row)
                            .copied()
                        {
                            let _ = toggle_tool_details(&mut state, entry_index);
                        }
                    }
                    _ => {}
                },
                Event::Key(key) => {
                    // 防御性检查：若审批通道已关闭则自动清除等待
                    if let Some(ref pending) = state.pending_approval {
                        if pending.responder.is_none() {
                            state.pending_approval = None;
                            state.status = state
                                .active_tool
                                .as_ref()
                                .map(|tool| format!("Running {tool}..."))
                                .or_else(|| Some("Thinking...".to_string()));
                        }
                    }
                    if state.pending_approval.is_some() {
                        let _ = handle_approval_key(&mut state, key);
                        continue;
                    }

                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        should_exit = true;
                        continue;
                    }

                    let visible_commands = get_visible_commands(&state.input);

                    match key {
                        KeyEvent {
                            code: KeyCode::Enter,
                            ..
                        } => {
                            if state.is_busy {
                                continue;
                            }
                            if !visible_commands.is_empty() {
                                let selected = visible_commands
                                    .get(state.selected_slash_index.min(visible_commands.len() - 1))
                                    .map(|x| x.0.clone())
                                    .unwrap_or(state.input.clone());
                                if state.input.trim() != selected {
                                    state.input = selected.to_string();
                                    state.cursor_offset = char_len(&state.input);
                                    state.selected_slash_index = 0;
                                    continue;
                                }
                            }
                            let submitted = state.input.clone();
                            state.input.clear();
                            state.cursor_offset = 0;
                            state.selected_slash_index = 0;
                            match handle_submit(&mut terminal, &mut state, submitted).await {
                                Ok(exit) => should_exit = exit,
                                Err(err) => {
                                    append_runtime_message(ChatMessage::Assistant {
                                        content: format!("submit failed: {err:#}"),
                                    });
                                    state.status = Some("Error".to_string());
                                    state.is_busy = false;
                                    state.active_tool = None;
                                    state.pending_approval = None;
                                    state.transcript_scroll_offset = 0;
                                }
                            }
                            state.message_count = runtime_messages_count();
                            state.turn_count += 1;
                        }
                        KeyEvent {
                            code: KeyCode::Backspace,
                            ..
                        } => {
                            if remove_char_before(&mut state.input, state.cursor_offset) {
                                state.cursor_offset -= 1;
                            }
                            state.selected_slash_index = 0;
                        }
                        KeyEvent {
                            code: KeyCode::Delete,
                            ..
                        } => {
                            let _ = remove_char_at(&mut state.input, state.cursor_offset);
                            state.selected_slash_index = 0;
                        }
                        KeyEvent {
                            code: KeyCode::Left,
                            ..
                        } => {
                            state.cursor_offset = state.cursor_offset.saturating_sub(1);
                        }
                        KeyEvent {
                            code: KeyCode::Right,
                            ..
                        } => {
                            state.cursor_offset =
                                (state.cursor_offset + 1).min(char_len(&state.input));
                        }
                        KeyEvent {
                            code: KeyCode::PageUp,
                            ..
                        } => {
                            let _ = scroll_transcript_by(&mut state, 8);
                        }
                        KeyEvent {
                            code: KeyCode::PageDown,
                            ..
                        } => {
                            let _ = scroll_transcript_by(&mut state, -8);
                        }
                        KeyEvent {
                            code: KeyCode::Tab, ..
                        } => {
                            if !visible_commands.is_empty()
                                && let Some(selected) = visible_commands
                                    .get(state.selected_slash_index.min(visible_commands.len() - 1))
                            {
                                state.input = selected.0.clone();
                                state.cursor_offset = char_len(&state.input);
                                state.selected_slash_index = 0;
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Up,
                            modifiers,
                            ..
                        } => {
                            if !visible_commands.is_empty() {
                                state.selected_slash_index =
                                    (state.selected_slash_index + visible_commands.len() - 1)
                                        % visible_commands.len();
                            } else if modifiers.contains(KeyModifiers::ALT) {
                                let _ = scroll_transcript_by(&mut state, 1);
                            } else {
                                let _ = history_up(&mut state);
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Down,
                            modifiers,
                            ..
                        } => {
                            if !visible_commands.is_empty() {
                                state.selected_slash_index =
                                    (state.selected_slash_index + 1) % visible_commands.len();
                            } else if modifiers.contains(KeyModifiers::ALT) {
                                let _ = scroll_transcript_by(&mut state, -1);
                            } else {
                                let _ = history_down(&mut state);
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Home,
                            ..
                        } => {
                            state.cursor_offset = 0;
                        }
                        KeyEvent {
                            code: KeyCode::End, ..
                        } => {
                            state.cursor_offset = char_len(&state.input);
                        }
                        KeyEvent {
                            code: KeyCode::Esc, ..
                        } => {
                            state.input.clear();
                            state.cursor_offset = 0;
                            state.selected_slash_index = 0;
                        }
                        KeyEvent {
                            code: KeyCode::Char('a'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            if state.input.is_empty() {
                                state.transcript_scroll_offset = state.session_max_scroll_offset;
                            } else {
                                state.cursor_offset = 0;
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Char('e'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            if state.input.is_empty() {
                                state.transcript_scroll_offset = 0;
                            } else {
                                state.cursor_offset = char_len(&state.input);
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Char('u'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            state.input.clear();
                            state.cursor_offset = 0;
                            state.selected_slash_index = 0;
                        }
                        KeyEvent {
                            code: KeyCode::Char('p'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            let _ = history_up(&mut state);
                        }
                        KeyEvent {
                            code: KeyCode::Char('n'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            let _ = history_down(&mut state);
                        }
                        KeyEvent {
                            code: KeyCode::Char(ch),
                            modifiers,
                            ..
                        } => {
                            if !modifiers.contains(KeyModifiers::CONTROL) {
                                let at = state.cursor_offset.min(char_len(&state.input));
                                insert_char_at(&mut state.input, at, ch);
                                state.cursor_offset = at + 1;
                                state.selected_slash_index = 0;
                                state.history_index = state.history.len();
                            }
                        }
                        _ => {}
                    }
                }
                Event::Paste(text) => {
                    if state.is_busy || state.pending_approval.is_some() {
                        continue;
                    }
                    // 多行粘贴：将换行符保持为文本，而非提交键
                    for ch in text.chars() {
                        let at = state.cursor_offset.min(char_len(&state.input));
                        insert_char_at(&mut state.input, at, ch);
                        state.cursor_offset = at + 1;
                    }
                    state.selected_slash_index = 0;
                    state.history_index = state.history.len();
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    let metadata = minicode_history::SessionMetadata {
        session_id: runtime_store().session_id.clone(),
        created_at: runtime_store().session_started_at.to_rfc3339(),
        ended_at: Some(chrono::Utc::now().to_rfc3339()),
        model: Some(runtime_config().model),
        cwd: runtime_store().cwd.clone(),
        turn_count: state.turn_count,
        user_input_count: state.message_count,
        tool_call_count: 0,
        status: "completed".to_string(),
    };

    let _ = minicode_history::save_session_metadata(&metadata);
    Ok(())
}
