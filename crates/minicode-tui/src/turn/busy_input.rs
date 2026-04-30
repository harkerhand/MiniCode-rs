use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

use crate::input::{
    char_len, get_visible_commands, history_down, history_up, insert_char_at, remove_char_at,
    remove_char_before, scroll_transcript_by, toggle_tool_details,
};
use crate::state::ScreenState;
use crate::turn::approval::handle_approval_key;

pub(crate) enum BusyEventAction {
    None,
    Submit(String),
    Interrupt,
}

/// 在模型忙碌期间处理允许的键鼠事件。
pub(crate) fn handle_busy_event(state: &mut ScreenState, event: Event) -> BusyEventAction {
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
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return BusyEventAction::Interrupt;
            }

            if state.pending_approval.is_some() && handle_approval_key(state, key) {
                return BusyEventAction::None;
            }

            let visible_commands = get_visible_commands(&state.input);
            match key {
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } => {
                    if !visible_commands.is_empty() {
                        let selected = visible_commands
                            .get(state.selected_slash_index.min(visible_commands.len() - 1))
                            .map(|x| x.0.clone())
                            .unwrap_or(state.input.clone());
                        if state.input.trim() != selected {
                            state.input = selected.to_string();
                            state.cursor_offset = char_len(&state.input);
                            state.selected_slash_index = 0;
                            return BusyEventAction::None;
                        }
                    }
                    let submitted = state.input.clone();
                    state.input.clear();
                    state.cursor_offset = 0;
                    state.selected_slash_index = 0;
                    if !submitted.trim().is_empty() {
                        return BusyEventAction::Submit(submitted);
                    }
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
                    state.cursor_offset = (state.cursor_offset + 1).min(char_len(&state.input));
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
                } => {
                    if !visible_commands.is_empty() {
                        state.selected_slash_index =
                            (state.selected_slash_index + visible_commands.len() - 1)
                                % visible_commands.len();
                    } else if modifiers.contains(KeyModifiers::ALT) {
                        let _ = scroll_transcript_by(state, 1);
                    } else {
                        let _ = history_up(state);
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
                        let _ = scroll_transcript_by(state, -1);
                    } else {
                        let _ = history_down(state);
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
                    let _ = history_up(state);
                }
                KeyEvent {
                    code: KeyCode::Char('n'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = history_down(state);
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
            // 多行粘贴：将换行符保持为文本，而非提交键
            for ch in text.chars() {
                let at = state.cursor_offset.min(char_len(&state.input));
                insert_char_at(&mut state.input, at, ch);
                state.cursor_offset = at + 1;
            }
            state.selected_slash_index = 0;
            state.history_index = state.history.len();
        }
        _ => {}
    }
    BusyEventAction::None
}
