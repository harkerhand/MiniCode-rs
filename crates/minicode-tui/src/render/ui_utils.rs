use ratatui::layout::{Constraint, Direction, Layout, Rect};
use unicode_width::UnicodeWidthStr;

pub(super) fn sanitize_line(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || *ch == '\t')
        .collect::<String>()
        .replace('\t', "    ")
}

pub(super) fn wrap_input_view(
    input: &str,
    cursor_offset: usize,
    text_width: usize,
) -> (Vec<String>, usize, usize) {
    let max_width = text_width.max(1);
    let chars = input.chars().collect::<Vec<_>>();
    let cursor = cursor_offset.min(chars.len());

    let mut lines = vec![String::new()];
    let mut row = 0usize;
    let mut col = 0usize;
    let mut cursor_row = 0usize;
    let mut cursor_col = 0usize;

    for (idx, ch) in chars.iter().enumerate() {
        if idx == cursor {
            cursor_row = row;
            cursor_col = col;
        }

        if *ch == '\n' {
            lines.push(String::new());
            row += 1;
            col = 0;
            continue;
        }

        let w = UnicodeWidthStr::width(ch.to_string().as_str()).max(1);
        if col + w > max_width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        lines[row].push(*ch);
        col += w;
    }

    if cursor == chars.len() {
        cursor_row = row;
        cursor_col = col;
    }

    if lines.is_empty() {
        lines.push(String::new());
    }
    (lines, cursor_row, cursor_col)
}

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
