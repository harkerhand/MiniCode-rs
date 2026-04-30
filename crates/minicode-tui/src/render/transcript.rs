use minicode_history::runtime_messages;
use minicode_types::ChatMessage;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::state::ScreenState;
use crate::theme::theme;

use super::markdown::render_markdown;
use super::ui_utils::sanitize_line;

const TOOL_PREVIEW_LINES: usize = 6;
const TOOL_PREVIEW_CHARS: usize = 180;

#[derive(Debug, Clone)]
struct TranscriptLine {
    kind: String,
    body: String,
}

fn transcript_from_messages() -> Vec<TranscriptLine> {
    runtime_messages()
        .into_iter()
        .filter_map(|msg| match msg {
            ChatMessage::System { .. }
            | ChatMessage::Minicode { .. }
            | ChatMessage::ContextSummary { .. } => None,
            ChatMessage::User { content } => Some(TranscriptLine {
                kind: "user".to_string(),
                body: content,
            }),
            ChatMessage::Assistant { content } => Some(TranscriptLine {
                kind: "assistant".to_string(),
                body: content,
            }),
            ChatMessage::AssistantProgress { content } => Some(TranscriptLine {
                kind: "progress".to_string(),
                body: content,
            }),
            ChatMessage::AssistantToolCall {
                tool_name, input, ..
            } => Some(TranscriptLine {
                kind: "tool".to_string(),
                body: format!("{}\n{}", tool_name, input),
            }),
            ChatMessage::ToolResult {
                content, is_error, ..
            } => Some(TranscriptLine {
                kind: if is_error {
                    "tool:error".to_string()
                } else {
                    "tool".to_string()
                },
                body: content,
            }),
            ChatMessage::Runtime {
                kind,
                content,
                flags,
            } => {
                if flags.contains(minicode_types::MessageFlags::DISPLAY) {
                    Some(TranscriptLine {
                        kind: format!("runtime:{kind}"),
                        body: content,
                    })
                } else {
                    None
                }
            }
        })
        .collect()
}

/// 按字符数截断字符串并在末尾追加省略符。
fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// 判断该转录条目是否属于工具输出。
fn is_tool_entry(entry: &TranscriptLine) -> bool {
    entry.kind == "tool" || entry.kind == "tool:error"
}

/// 根据消息类型生成标题行样式。
fn transcript_title_line(kind: &str) -> Line<'static> {
    let theme = theme();
    let (label, style) = match kind {
        "assistant" => ("assistant", theme.assistant_style()),
        "user" => ("you", theme.user_style()),
        "minicode" => ("minicode", theme.progress_style()),
        "progress" => ("progress", theme.progress_style()),
        "tool:error" => ("tool err", theme.tool_error_style()),
        "tool" => ("tool", theme.tool_style()),
        "runtime:command" => ("cmd", theme.user_style()),
        "runtime:command:result" => ("cmd out", theme.assistant_style()),
        "runtime:command:error" => ("cmd err", theme.tool_error_style()),
        _ => (kind, Style::default().fg(Color::Gray)),
    };
    Line::from(vec![
        Span::styled("▌", style),
        Span::raw(" "),
        Span::styled(label.to_string(), style.add_modifier(Modifier::BOLD)),
    ])
}

pub(super) struct SessionRender {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) toggle_targets: Vec<(usize, usize)>,
}

/// 按显示宽度将文本换行并附加统一前缀。
fn wrapped_prefixed_lines(text: &str, prefix: &str, width: usize) -> Vec<Line<'static>> {
    let safe_width = width.max(1);
    let prefix_width = UnicodeWidthStr::width(prefix).min(safe_width.saturating_sub(1));
    let content_width = safe_width.saturating_sub(prefix_width).max(1);

    let mut out = Vec::new();
    for raw in text.split('\n') {
        let mut current = String::new();
        let mut used = 0usize;
        for ch in raw.chars() {
            let w = UnicodeWidthStr::width(ch.to_string().as_str()).max(1);
            if used + w > content_width {
                out.push(Line::from(format!("{}{}", prefix, current)));
                current.clear();
                used = 0;
            }
            current.push(ch);
            used += w;
        }
        out.push(Line::from(format!("{}{}", prefix, current)));
    }
    if out.is_empty() {
        out.push(Line::from(prefix.to_string()));
    }
    out
}

/// 将会话转录渲染为可显示的行集合。
pub(super) fn transcript_lines(state: &ScreenState, width: usize) -> SessionRender {
    let theme = theme();
    let mut lines = Vec::new();
    let mut toggle_targets = Vec::new();
    let transcript = transcript_from_messages();
    for (idx, entry) in transcript.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(""));
        }

        if is_tool_entry(entry) {
            let expanded = state.expanded_tool_entries.contains(&idx);
            let body_lines = entry.body.lines().collect::<Vec<_>>();
            let total = body_lines.len();
            let mut truncated = false;
            let collapsed_preview_len = total.min(TOOL_PREVIEW_LINES);
            let collapsible_by_lines = total > collapsed_preview_len;
            let mut collapsible_by_chars = false;
            if !collapsible_by_lines {
                for line in body_lines.iter().take(collapsed_preview_len) {
                    if sanitize_line(line).chars().count() > TOOL_PREVIEW_CHARS {
                        collapsible_by_chars = true;
                        break;
                    }
                }
            }
            let can_toggle = collapsible_by_lines || collapsible_by_chars;
            let preview_len = if expanded {
                total
            } else {
                total.min(TOOL_PREVIEW_LINES)
            };
            let hidden = total.saturating_sub(preview_len);
            let style = if entry.kind == "tool:error" {
                theme.tool_error_style()
            } else {
                theme.tool_style()
            };
            let title_line_idx = lines.len();
            let mut title_spans = vec![
                Span::styled("▌", style),
                Span::raw(" "),
                Span::styled(
                    if entry.kind == "tool:error" {
                        "tool err"
                    } else {
                        "tool"
                    }
                    .to_string(),
                    style.add_modifier(Modifier::BOLD),
                ),
            ];
            if can_toggle {
                title_spans.push(Span::raw("  "));
                title_spans.push(Span::styled(
                    if expanded { "[收起]" } else { "[展开]" },
                    theme.expandable_style(),
                ));
                toggle_targets.push((title_line_idx, idx));
            }
            lines.push(Line::from(title_spans));

            for line in body_lines.iter().take(preview_len) {
                let sanitized = sanitize_line(line);
                let display = if expanded {
                    sanitized
                } else {
                    let clipped = truncate_chars(&sanitized, TOOL_PREVIEW_CHARS);
                    if clipped != sanitized {
                        truncated = true;
                    }
                    clipped
                };
                lines.extend(wrapped_prefixed_lines(&display, "  ", width));
            }
            if !expanded && hidden == 0 && truncated {
                lines.push(Line::from("  ..."));
            }
        } else {
            lines.push(transcript_title_line(&entry.kind));
            match entry.kind.as_str() {
                "assistant" | "progress" => {
                    // Markdown 渲染
                    for md_line in render_markdown(&entry.body) {
                        let mut spans = vec![Span::raw("  ")];
                        spans.extend(md_line.into_iter());
                        lines.push(Line::from(spans));
                    }
                }
                _ => {
                    for line in entry.body.lines() {
                        lines.extend(wrapped_prefixed_lines(
                            &sanitize_line(line),
                            "  ",
                            width,
                        ));
                    }
                }
            }
        }
    }
    SessionRender {
        lines,
        toggle_targets,
    }
}

/// 构建会话区域的最终渲染行。
pub(super) fn session_lines(state: &ScreenState, width: usize) -> SessionRender {
    transcript_lines(state, width)
}
