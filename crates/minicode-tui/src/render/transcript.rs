use minicode_background_tasks::list_background_tasks;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::{ScreenState, TranscriptEntry};

use super::ui_utils::sanitize_line;

fn transcript_title_line(kind: &str) -> Line<'static> {
    let (label, color) = match kind {
        "assistant" => ("assistant", Color::Green),
        "user" => ("you", Color::Cyan),
        "progress" => ("progress", Color::Yellow),
        "tool:error" => ("tool err", Color::Red),
        "tool" => ("tool", Color::Magenta),
        _ => (kind, Color::Gray),
    };
    Line::from(vec![
        Span::styled("▌", Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

pub(super) fn transcript_lines(entries: &[TranscriptEntry]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(""));
        }
        lines.push(transcript_title_line(&entry.kind));
        for line in entry.body.lines() {
            lines.push(Line::from(format!("  {}", sanitize_line(line))));
        }
    }
    lines
}

pub(super) fn session_lines(state: &ScreenState) -> Vec<Line<'static>> {
    let mut lines = transcript_lines(&state.transcript);
    let has_transcript = !lines.is_empty();

    if has_transcript {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![
        Span::styled(
            "▌",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "activity",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if let Some(tool) = &state.active_tool {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "running",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::raw(tool.clone()),
        ]));
    }

    if state.recent_tools.is_empty() {
        lines.push(Line::from("  recent none"));
    } else {
        for (name, ok) in state.recent_tools.iter().rev().take(6) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    if *ok { "ok" } else { "err" },
                    Style::default()
                        .fg(if *ok { Color::Green } else { Color::Red })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::raw(name.clone()),
            ]));
        }
    }

    let tasks = list_background_tasks();
    if !tasks.is_empty() {
        lines.push(Line::from("  "));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "background",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for task in tasks.iter().rev().take(4) {
            let color = match task.status.as_str() {
                "running" => Color::Yellow,
                "completed" => Color::Green,
                _ => Color::Red,
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    task.status.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::raw(format!("pid={} {}", task.pid, task.command)),
            ]));
        }
    }

    lines
}
