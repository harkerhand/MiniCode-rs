use std::io::Stdout;

use anyhow::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListState, Paragraph, Wrap};

use crate::input::{display_width, get_visible_commands};
use crate::state::{ScreenState, TuiAppArgs};

mod approval;
mod header;
mod transcript;
mod ui_utils;

use approval::build_approval_lines;
use header::build_header_lines;
use transcript::session_lines;
use ui_utils::{centered_rect, input_viewport, sanitize_line};

pub(crate) fn render_screen(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    args: &TuiAppArgs,
    state: &ScreenState,
) -> Result<()> {
    let visible_commands = get_visible_commands(&state.input);
    let command_rows = if visible_commands.is_empty() {
        0u16
    } else {
        (visible_commands.len().min(6) + 2) as u16
    };

    terminal.draw(|frame| {
        let area = frame.area();
        let input_height = if command_rows > 0 { 11 } else { 5 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
                Constraint::Min(10),
                Constraint::Length(input_height),
            ])
            .split(area);

        let header = Paragraph::new(build_header_lines(args, state))
            .block(
                Block::default()
                    .title(" Workspace ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(Style::default().fg(Color::LightCyan)),
            )
            .wrap(Wrap { trim: true });
        frame.render_widget(header, chunks[0]);

        let feed_lines = session_lines(state);
        let feed_line_count = feed_lines.len();
        let feed_viewport_height = chunks[1].height.saturating_sub(2) as usize;
        let max_scroll = feed_line_count.saturating_sub(feed_viewport_height);
        let scroll_from_bottom = state.transcript_scroll_offset.min(max_scroll);
        let scroll_from_top = max_scroll.saturating_sub(scroll_from_bottom);
        let fallback = vec![Line::from(
            "(no messages yet, enter /help to list commands)",
        )];
        let feed = Paragraph::new(if feed_lines.is_empty() {
            fallback
        } else {
            feed_lines
        })
        .block(
            Block::default()
                .title(" Session ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::Blue)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_from_top as u16, 0));
        frame.render_widget(feed, chunks[1]);

        let prompt_input = sanitize_line(&state.input);
        let input_box = chunks[2];
        let available_input_width = input_box.width.saturating_sub(14) as usize;
        let (display_input, cursor_dx) = input_viewport(
            &prompt_input,
            state.cursor_offset,
            available_input_width.max(1),
        );

        let prompt_text = vec![
            Line::from(format!(
                "status: {}{}{}{}{}",
                state.status.clone().unwrap_or_else(|| "Ready".to_string()),
                state
                    .active_tool
                    .as_ref()
                    .map(|x| format!(" | active={}", x))
                    .unwrap_or_default(),
                if state.is_busy {
                    " | busy".to_string()
                } else {
                    String::new()
                },
                if state.transcript_scroll_offset > 0 {
                    format!(" | scroll={}", state.transcript_scroll_offset)
                } else {
                    String::new()
                },
                {
                    let running_shells = minicode_background_tasks::list_background_tasks()
                        .into_iter()
                        .filter(|task| task.status == "running")
                        .count();
                    format!(" | tools=on | skills=on | shells={}", running_shells)
                }
            ))
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(vec![
                Span::styled(
                    "mini-code> ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(display_input),
            ]),
            Line::from(Span::styled(
                "Enter submit | Tab complete | PgUp/PgDn scroll | Ctrl+C exit",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let prompt = Paragraph::new(prompt_text)
            .block(
                Block::default()
                    .title(" Input ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(Style::default().fg(Color::Green)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(prompt, input_box);

        if command_rows > 0 {
            let command_area = Rect {
                x: input_box.x + 1,
                y: input_box.y + 3,
                width: input_box.width.saturating_sub(2),
                height: input_box.height.saturating_sub(4),
            };
            let items = visible_commands
                .iter()
                .take(6)
                .map(|cmd| {
                    ratatui::widgets::ListItem::new(Line::from(vec![
                        Span::styled(
                            cmd.usage.to_string(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::raw(cmd.description.to_string()),
                    ]))
                })
                .collect::<Vec<_>>();

            let mut list_state = ListState::default();
            list_state.select(Some(
                state
                    .selected_slash_index
                    .min(visible_commands.len().min(6) - 1),
            ));
            let commands = List::new(items)
                .highlight_style(
                    Style::default()
                        .bg(Color::Rgb(30, 50, 80))
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");
            frame.render_stateful_widget(commands, command_area, &mut list_state);
        }

        let prompt_area: Rect = input_box;
        let prefix_width = display_width("mini-code> ") as u16;
        let cursor_x = (prompt_area.x + 1 + prefix_width + cursor_dx as u16)
            .min(prompt_area.x + prompt_area.width.saturating_sub(2));
        let cursor_y =
            (prompt_area.y + 2).min(prompt_area.y + prompt_area.height.saturating_sub(1));

        if let Some(pending) = &state.pending_approval {
            let popup = centered_rect(70, 45, area);
            frame.render_widget(Clear, popup);
            let lines = build_approval_lines(pending);
            let dialog = Paragraph::new(lines)
                .block(
                    Block::default()
                        .title(" Approval Required ")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .style(Style::default().fg(Color::LightRed)),
                )
                .wrap(Wrap { trim: true });
            frame.render_widget(dialog, popup);
        }

        frame.set_cursor_position((cursor_x, cursor_y));
    })?;
    Ok(())
}
