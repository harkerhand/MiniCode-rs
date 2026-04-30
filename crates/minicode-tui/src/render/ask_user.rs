use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::PendingAskUser;

pub(super) fn build_ask_user_lines(pending: &PendingAskUser) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            pending.question.clone(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (idx, opt) in pending.options.iter().enumerate() {
        let selected = idx == pending.selected_index;
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "▶" } else { " " },
                Style::default().fg(Color::LightBlue),
            ),
            Span::raw(" "),
            Span::styled(
                format!("({})", idx + 1),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                opt.clone(),
                Style::default().fg(Color::White).add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Arrow/Tab to move, number key to pick, Enter confirm, Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}
