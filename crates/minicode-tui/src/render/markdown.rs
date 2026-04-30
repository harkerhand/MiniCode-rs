use ratatui::text::{Line, Span};

use crate::theme::theme;

/// 将 markdown 文本渲染为 ratatui Line 列表。
pub(crate) fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let theme = theme();
    let mut lines: Vec<Line> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        // 代码块边界
        if trimmed.starts_with("```") {
            if in_code_block {
                // 结束代码块
                render_code_block(&mut lines, &code_lang, &code_lines, &theme);
                code_lang.clear();
                code_lines.clear();
                in_code_block = false;
            } else {
                // 开始代码块
                in_code_block = true;
                code_lang = trimmed.strip_prefix("```").unwrap_or("").trim().to_string();
            }
            continue;
        }

        if in_code_block {
            code_lines.push(raw_line.to_string());
            continue;
        }

        // 行内代码（反引号）
        if trimmed.starts_with("`") && trimmed.ends_with("`") && trimmed.len() > 2 {
            let code = &trimmed[1..trimmed.len() - 1];
            lines.push(Line::from(Span::styled(
                code.to_string(),
                theme.code_inline_style(),
            )));
            continue;
        }

        // 标题
        if let Some(heading) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                heading.to_string(),
                theme.heading1_style(),
            )));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                heading.to_string(),
                theme.heading2_style(),
            )));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                heading.to_string(),
                theme.heading3_style(),
            )));
            continue;
        }

        // 水平线
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            lines.push(Line::from(Span::styled(
                "─".repeat(40),
                theme.header_label_info_style(),
            )));
            continue;
        }

        // 引用
        if let Some(quote) = trimmed.strip_prefix("> ") {
            lines.push(Line::from(Span::styled(
                format!("  {quote}"),
                theme.header_label_session_style(),
            )));
            continue;
        }

        // 无序列表
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let mut spans = vec![Span::styled(
                "  • ",
                theme.header_label_session_style(),
            )];
            render_inline_spans(item, &theme, &mut spans);
            lines.push(Line::from(spans));
            continue;
        }

        // 有序列表
        if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
            if rest.starts_with(". ") {
                let item = &rest[2..];
                let num_part = trimmed.split('.').next().unwrap_or("");
                let mut spans = vec![Span::styled(
                    format!("  {num_part}. "),
                    theme.header_label_session_style(),
                )];
                render_inline_spans(item, &theme, &mut spans);
                lines.push(Line::from(spans));
                continue;
            }
        }

        // 空行
        if trimmed.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        // 普通段落：解析行内格式
        let mut inline_spans = Vec::new();
        render_inline_spans(raw_line, &theme, &mut inline_spans);
        lines.push(Line::from(inline_spans));
    }

    // 处理未闭合的代码块
    if in_code_block && !code_lines.is_empty() {
        render_code_block(&mut lines, &code_lang, &code_lines, &theme);
    }

    lines
}

/// 渲染代码块。
fn render_code_block(
    lines: &mut Vec<Line<'static>>,
    lang: &str,
    code_lines: &[String],
    theme: &crate::theme::Theme,
) {
    let title = if lang.is_empty() {
        " code ".to_string()
    } else {
        format!(" {lang} ")
    };
    lines.push(Line::from(Span::styled(title, theme.code_block_header_style())));
    for cl in code_lines {
        lines.push(Line::from(Span::styled(
            format!("  {cl}"),
            theme.code_block_style(),
        )));
    }
}

/// 解析行内 markdown：粗体、斜体、代码、链接，追加到给定 spans 列表。
fn render_inline_spans(
    text: &str,
    theme: &crate::theme::Theme,
    out: &mut Vec<Span<'static>>,
) {
    let mut rest = text;
    let mut current = String::new();
    let count_before = out.len();

    while let Some(pos) = rest.find(['*', '_', '`', '[']) {
        current.push_str(&rest[..pos]);
        rest = &rest[pos..];

        // 行内代码
        if rest.starts_with('`') && !rest.starts_with("``") {
            rest = &rest[1..];
            if let Some(end) = rest.find('`') {
                if !current.is_empty() {
                    out.push(Span::raw(std::mem::take(&mut current)));
                }
                out.push(Span::styled(
                    rest[..end].to_string(),
                    theme.code_inline_style(),
                ));
                rest = &rest[end + 1..];
                continue;
            }
        }

        // 粗体 **text**
        if rest.starts_with("**") {
            rest = &rest[2..];
            if let Some(end) = rest.find("**") {
                if !current.is_empty() {
                    out.push(Span::raw(std::mem::take(&mut current)));
                }
                out.push(Span::styled(
                    rest[..end].to_string(),
                    theme.bold_style(),
                ));
                rest = &rest[end + 2..];
                continue;
            }
        }

        // 斜体 *text*
        if rest.starts_with('*') {
            rest = &rest[1..];
            if let Some(end) = rest.find('*') {
                if !current.is_empty() {
                    out.push(Span::raw(std::mem::take(&mut current)));
                }
                out.push(Span::styled(
                    rest[..end].to_string(),
                    theme.italic_style(),
                ));
                rest = &rest[end + 1..];
                continue;
            }
        }

        // 链接 [text](url) — 只保留文本
        if rest.starts_with('[') {
            let link_start = &rest[1..];
            if let Some(bracket_end) = link_start.find("](") {
                let link_text = &link_start[..bracket_end];
                let after_bracket = &link_start[bracket_end + 2..];
                if let Some(paren_end) = after_bracket.find(')') {
                    current.push_str(link_text);
                    rest = &after_bracket[paren_end + 1..];
                    continue;
                }
            }
        }

        // 未匹配的特殊字符，作为普通文本
        let ch = rest.chars().next().unwrap_or(' ');
        current.push(ch);
        rest = &rest[ch.len_utf8()..];
    }

    current.push_str(rest);
    if !current.is_empty() || out.len() == count_before {
        out.push(Span::raw(current));
    }
}
