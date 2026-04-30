use crossterm::event::{KeyCode, KeyEvent};

use crate::state::ScreenState;

pub(crate) enum AskUserAction {
    None,
    Handled,
    Submit(String),
    Cancelled,
}

pub(crate) fn handle_ask_user_key(state: &mut ScreenState, key: KeyEvent) -> AskUserAction {
    let Some(pending) = state.pending_ask_user.as_mut() else {
        return AskUserAction::None;
    };
    let len = pending.options.len();
    if len == 0 {
        return AskUserAction::None;
    }
    match key.code {
        KeyCode::Left | KeyCode::Up => {
            pending.selected_index = if pending.selected_index == 0 {
                len - 1
            } else {
                pending.selected_index - 1
            };
            AskUserAction::Handled
        }
        KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
            pending.selected_index = (pending.selected_index + 1) % len;
            AskUserAction::Handled
        }
        KeyCode::Char(ch) if ch.is_ascii_digit() => {
            let idx = (ch as u8).saturating_sub(b'1') as usize;
            if idx < len {
                pending.selected_index = idx;
                AskUserAction::Handled
            } else {
                AskUserAction::None
            }
        }
        KeyCode::Enter => {
            let selected = pending.options[pending.selected_index].clone();
            state.pending_ask_user = None;
            AskUserAction::Submit(selected)
        }
        KeyCode::Esc => {
            state.pending_ask_user = None;
            AskUserAction::Cancelled
        }
        _ => AskUserAction::None,
    }
}
