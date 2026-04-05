use crate::Tool;
use crate::web::WEB_USER_AGENT;
use crate::web::decode_html;
use crate::web::extract_between;
use crate::web::strip_tags;
use anyhow::Result;
use async_trait::async_trait;
use minicode_tool::ToolContext;
use minicode_tool::ToolResult;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Default)]
pub struct WebFetchTool;
#[derive(Debug, Deserialize)]
struct WebFetchInput {
    url: String,
    max_chars: Option<usize>,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page and extract readable text content."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "url":{"type":"string","description":"HTTP or HTTPS URL to fetch."},
                "max_chars":{"type":"number","description":"Maximum number of characters to return. Defaults to 12000."}
            },
            "required":["url"]
        })
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let parsed: WebFetchInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let max_chars = parsed.max_chars.unwrap_or(12_000).max(500);
        if parsed.url.trim().is_empty() {
            return ToolResult::err("url is required");
        }

        match fetch_web_page(&parsed.url, max_chars).await {
            Ok(page) => {
                if page.status >= 400 {
                    return ToolResult::err(format!(
                        "HTTP {} {}: {}",
                        page.status, page.status_text, parsed.url
                    ));
                }
                let mut lines = vec![
                    format!("URL: {}", page.final_url),
                    format!("STATUS: {}", page.status),
                    format!("CONTENT_TYPE: {}", page.content_type),
                ];
                if let Some(title) = page.title
                    && !title.is_empty()
                {
                    lines.push(format!("TITLE: {}", title));
                }
                lines.push(String::new());
                lines.push(page.content);
                ToolResult::ok(lines.join("\n"))
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
struct FetchedPage {
    final_url: String,
    status: u16,
    status_text: String,
    content_type: String,
    title: Option<String>,
    content: String,
}

async fn fetch_web_page(url: &str, max_chars: usize) -> Result<FetchedPage> {
    let client = reqwest::Client::builder()
        .user_agent(WEB_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8,*/*;q=0.7",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .send()
        .await?;

    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("Unknown")
        .to_string();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response.text().await?;

    if content_type.to_ascii_lowercase().contains("html") {
        Ok(FetchedPage {
            final_url,
            status,
            status_text,
            content_type,
            title: extract_title(&text),
            content: truncate_chars(&extract_readable_text(&text), max_chars),
        })
    } else {
        Ok(FetchedPage {
            final_url,
            status,
            status_text,
            content_type,
            title: None,
            content: truncate_chars(&text, max_chars),
        })
    }
}

fn extract_readable_text(html: &str) -> String {
    let mut text = html.to_string();
    for (start, end) in [
        ("<script", "</script>"),
        ("<style", "</style>"),
        ("<noscript", "</noscript>"),
        ("<svg", "</svg>"),
    ] {
        text = remove_block_like(&text, start, end);
    }
    text = strip_tags(&text);
    decode_html(&text)
}

fn extract_title(html: &str) -> Option<String> {
    extract_between(html, "<title", "</title>").map(|raw| {
        let title_text = raw
            .split_once('>')
            .map(|(_, right)| right)
            .unwrap_or(raw.as_str());
        strip_tags(&decode_html(title_text))
    })
}

fn remove_block_like(text: &str, start_tag: &str, end_tag: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start_pos) = rest
            .to_ascii_lowercase()
            .find(&start_tag.to_ascii_lowercase())
        else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start_pos]);
        let after_start = &rest[start_pos..];
        let Some(end_pos_rel) = after_start
            .to_ascii_lowercase()
            .find(&end_tag.to_ascii_lowercase())
        else {
            break;
        };
        let end_pos = start_pos + end_pos_rel + end_tag.len();
        rest = &rest[end_pos..];
        out.push(' ');
    }
    out
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect::<String>()
}
