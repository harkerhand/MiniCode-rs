use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

use crate::input::{scroll_transcript_by, toggle_tool_details};
use crate::state::ScreenState;
use crate::turn::approval::handle_approval_key;

/// 在模型忙碌期间处理允许的键鼠事件。
pub(crate) fn handle_busy_event(state: &mut ScreenState, event: Event) {
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
