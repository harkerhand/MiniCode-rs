use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use minicode_permissions::{PermissionDecision, PermissionPromptResult};

use crate::state::ScreenState;

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
    let restore_status_after_approval = |state: &mut ScreenState| {
        state.status = state
            .active_tool
            .as_ref()
            .map(|tool| format!("Running {tool}..."))
            .or_else(|| Some("Thinking...".to_string()));
    };

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
                restore_status_after_approval(state);
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
            restore_status_after_approval(state);
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
            restore_status_after_approval(state);
            true
        }
        _ => false,
    }
}
