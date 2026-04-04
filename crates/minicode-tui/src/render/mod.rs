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
use crate::theme::theme;

mod approval;
mod header;
mod transcript;
mod ui_utils;

use approval::build_approval_lines;
use header::build_header_lines;
use transcript::session_lines;
use ui_utils::{centered_rect, sanitize_line, wrap_input_view};

pub(crate) fn render_screen(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    args: &TuiAppArgs,
    state: &mut ScreenState,
) -> Result<()> {
    let theme = theme();
    let visible_commands = get_visible_commands(&state.input);
    const MAX_COMMAND_ROWS: usize = 8;

    terminal.draw(|frame| {
        let area = frame.area();
        let prefix_width = display_width("mini-code> ");
        let input_inner_width = area.width.saturating_sub(2) as usize;
        let input_text_width = input_inner_width.saturating_sub(prefix_width).max(1);
        let prompt_input = sanitize_line(&state.input);
        let (wrapped_input_lines, cursor_row, cursor_col) =
            wrap_input_view(&prompt_input, state.cursor_offset, input_text_width);
        let visible_command_rows = if visible_commands.is_empty() {
            0usize
        } else {
            visible_commands.len().min(MAX_COMMAND_ROWS)
        };
        let input_height = (wrapped_input_lines.len() + visible_command_rows + 2)
            .clamp(3, area.height.saturating_sub(6).max(3) as usize)
            as u16;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
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
                    .style(theme.header_style()),
            )
            .wrap(Wrap { trim: true });
        frame.render_widget(header, chunks[0]);

        state.visible_tool_toggle_rows.clear();
        let session_inner_width = chunks[1].width.saturating_sub(2) as usize;
        let session_render = session_lines(state, session_inner_width);
        let feed_line_count = session_render.lines.len();
        let feed_viewport_height = chunks[1].height.saturating_sub(2) as usize;
        let max_scroll = feed_line_count.saturating_sub(feed_viewport_height);
        state.session_max_scroll_offset = max_scroll;
        state.transcript_scroll_offset = state.transcript_scroll_offset.min(max_scroll);
        let scroll_from_bottom = state.transcript_scroll_offset;
        let scroll_from_top = max_scroll.saturating_sub(scroll_from_bottom);
        for (line_idx, entry_idx) in &session_render.toggle_targets {
            if *line_idx >= scroll_from_top && *line_idx < scroll_from_top + feed_viewport_height {
                let row = (*line_idx - scroll_from_top) as u16;
                let screen_y = chunks[1].y + 1 + row;
                if screen_y < chunks[1].y + chunks[1].height.saturating_sub(1) {
                    state.visible_tool_toggle_rows.push((screen_y, *entry_idx));
                }
            }
        }
        let fallback = vec![Line::from(
            "(no messages yet, enter /help to list commands)",
        )];
        let feed_lines = if session_render.lines.is_empty() {
            fallback
        } else {
            session_render.lines
        };
        let scroll_title = if state.transcript_scroll_offset > 0 {
            format!(" Session (scroll: {}) ", state.transcript_scroll_offset)
        } else {
            " Session ".to_string()
        };
        let feed = Paragraph::new(feed_lines)
            .block(
                Block::default()
                    .title(scroll_title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(theme.session_style()),
            )
            .scroll((scroll_from_top as u16, 0));
        frame.render_widget(feed, chunks[1]);

        let status_text = state.status.clone().unwrap_or_else(|| "Ready".to_string());

        let mut status_info = String::from("status: ");
        status_info.push_str(&status_text);

        if let Some(active_tool) = &state.active_tool {
            status_info.push_str(" | active=");
            status_info.push_str(active_tool);
        }

        let input_box = chunks[2];
        let mut prompt_text = Vec::with_capacity(wrapped_input_lines.len());
        for (idx, line) in wrapped_input_lines.iter().enumerate() {
            let prefix = if idx == 0 {
                "mini-code> ".to_string()
            } else {
                " ".repeat(prefix_width)
            };
            prompt_text.push(Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(theme.input)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(line.clone()),
            ]));
        }

        // Build block with status on the right
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(theme.input_style());

        // Create title string with left-aligned "Input"
        let input_title = " Input ";
        block = block.title(input_title);

        // Create right-aligned status title
        let status_title = format!(" {} ", status_info);
        block = block.title(status_title);
        let prompt = Paragraph::new(prompt_text)
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(prompt, input_box);

        if !visible_commands.is_empty() {
            // Render command list inside the input box to avoid drawing beyond terminal bounds.
            let used_rows = wrapped_input_lines.len();
            let max_items = input_box.height.saturating_sub((used_rows + 2) as u16) as usize;
            if max_items > 0 {
                let page_size = max_items.min(MAX_COMMAND_ROWS);
                let selected = state.selected_slash_index.min(visible_commands.len() - 1);
                let page_start = (selected / page_size) * page_size;
                let page_end = (page_start + page_size).min(visible_commands.len());

                let command_area = Rect {
                    x: input_box.x + 1,
                    y: input_box.y + 1 + used_rows as u16,
                    width: input_box.width.saturating_sub(2),
                    height: (page_end - page_start) as u16,
                };
                let items = visible_commands
                    .iter()
                    .skip(page_start)
                    .take(page_end - page_start)
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
                list_state.select(Some(selected - page_start));
                let commands = List::new(items)
                    .highlight_style(
                        Style::default()
                            .bg(theme.command_highlight)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("▶ ");
                frame.render_stateful_widget(commands, command_area, &mut list_state);
            }
        }

        let prompt_area: Rect = input_box;
        let cursor_x = (prompt_area.x + 1 + prefix_width as u16 + cursor_col as u16)
            .min(prompt_area.x + prompt_area.width.saturating_sub(2));
        let cursor_y = (prompt_area.y + 1 + cursor_row as u16)
            .min(prompt_area.y + prompt_area.height.saturating_sub(2));

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
                        .style(theme.approval_style()),
                )
                .wrap(Wrap { trim: true });
            frame.render_widget(dialog, popup);
        }

        frame.set_cursor_position((cursor_x, cursor_y));
    })?;
    Ok(())
}
