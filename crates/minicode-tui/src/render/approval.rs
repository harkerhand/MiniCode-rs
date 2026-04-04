use minicode_permissions::{PermissionDecision, PermissionPromptKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::PendingApproval;

use super::ui_utils::sanitize_line;

const MAX_TITLE_CHARS: usize = 84;
const MAX_DETAIL_CHARS: usize = 96;
const MAX_SCOPE_CHARS: usize = 84;
const MAX_FEEDBACK_CHARS: usize = 96;
const MAX_DETAIL_LINES: usize = 8;

fn truncate_for_dialog(input: &str, max_chars: usize) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return chars.into_iter().take(max_chars).collect();
    }
    let kept: String = chars.into_iter().take(max_chars - 3).collect();
    format!("{kept}...")
}

pub(super) fn build_approval_lines(pending: &PendingApproval) -> Vec<Line<'static>> {
    let kind = match pending.request.kind {
        PermissionPromptKind::Path => "PATH",
        PermissionPromptKind::Command => "COMMAND",
        PermissionPromptKind::Edit => "EDIT",
    };
    let mut lines = vec![Line::from(vec![Span::styled(
        format!(
            "[{kind}] {}",
            truncate_for_dialog(&sanitize_line(&pending.request.title), MAX_TITLE_CHARS)
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )])];
    lines.push(Line::from(""));
    for detail in pending.request.details.iter().take(MAX_DETAIL_LINES) {
        lines.push(Line::from(format!(
            "- {}",
            truncate_for_dialog(&sanitize_line(detail), MAX_DETAIL_CHARS)
        )));
    }
    if pending.request.details.len() > MAX_DETAIL_LINES {
        lines.push(Line::from(Span::styled(
            format!(
                "... {} more detail lines omitted",
                pending.request.details.len() - MAX_DETAIL_LINES
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(format!(
        "- scope: {}",
        truncate_for_dialog(&sanitize_line(&pending.request.scope), MAX_SCOPE_CHARS)
    )));
    lines.push(Line::from(""));
    for (idx, choice) in pending.request.choices.iter().enumerate() {
        let selected = idx == pending.selected_index;
        let color = match choice.decision {
            PermissionDecision::AllowOnce
            | PermissionDecision::AllowAlways
            | PermissionDecision::AllowTurn
            | PermissionDecision::AllowAllTurn => Color::Green,
            PermissionDecision::DenyWithFeedback => Color::LightYellow,
            PermissionDecision::DenyOnce | PermissionDecision::DenyAlways => Color::Red,
        };
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "▶" } else { " " },
                Style::default().fg(Color::LightBlue),
            ),
            Span::raw(" "),
            Span::styled(
                format!("({})", choice.key),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                choice.label.clone(),
                Style::default().fg(color).add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
        ]));
    }

    if pending.awaiting_feedback {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Enter guidance feedback (Enter to submit, Esc to cancel):",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            truncate_for_dialog(&sanitize_line(&pending.feedback), MAX_FEEDBACK_CHARS),
            Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 65)),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Arrow/Tab to move, number key to pick, Enter confirm, Esc deny",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}
