use anyhow::Result;
use async_trait::async_trait;
use minicode_tool::{Tool, ToolResult};
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;

use crate::web::WEB_USER_AGENT;
use crate::web::decode_html;
use crate::web::extract_between;
use crate::web::strip_tags;

#[derive(Default)]
pub struct WebSearchTool;
#[derive(Debug, Deserialize)]
struct WebSearchInput {
    query: String,
    max_results: Option<usize>,
    allowed_domains: Option<Vec<String>>,
    blocked_domains: Option<Vec<String>>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web using DuckDuckGo Lite with a China-friendly fallback."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "query":{"type":"string","description":"Search query."},
                "max_results":{"type":"number","description":"Maximum number of results to return. Defaults to 5."},
                "allowed_domains":{"type":"array","items":{"type":"string"},"description":"Only return results from these domains."},
                "blocked_domains":{"type":"array","items":{"type":"string"},"description":"Exclude results from these domains."}
            },
            "required":["query"]
        })
    }

    async fn run(&self, input: Value) -> ToolResult {
        let parsed: WebSearchInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let query = parsed.query.trim();
        if query.is_empty() {
            return ToolResult::err("query is required");
        }

        match search_duckduckgo_lite(
            query,
            parsed.max_results.unwrap_or(5),
            parsed.allowed_domains.unwrap_or_default(),
            parsed.blocked_domains.unwrap_or_default(),
        )
        .await
        {
            Ok(items) => {
                if items.is_empty() {
                    return ToolResult::ok("No results found.");
                }
                let mut lines = vec![format!("QUERY: {query}"), String::new()];
                for (idx, item) in items.iter().enumerate() {
                    lines.push(format!("[{}] {}", idx + 1, item.title));
                    lines.push(format!("    URL: {}", item.link));
                    if !item.snippet.is_empty() {
                        lines.push(format!("    {}", item.snippet));
                    }
                    lines.push(String::new());
                }
                ToolResult::ok(lines.join("\n").trim_end().to_string())
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

async fn search_duckduckgo_lite(
    query: &str,
    max_results: usize,
    allowed_domains: Vec<String>,
    blocked_domains: Vec<String>,
) -> Result<Vec<WebSearchResult>> {
    let client = reqwest::Client::builder()
        .user_agent(WEB_USER_AGENT)
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .build()?;

    let allowed = normalize_domain_list(&allowed_domains);
    let blocked = normalize_domain_list(&blocked_domains);
    let limit = max_results.clamp(1, 20);
    let mut errors = vec![];

    match search_ddg_with_client(&client, query).await {
        Ok(mut parsed) => {
            parsed.retain(|r| passes_domain_filter(&r.link, &allowed, &blocked));
            if !parsed.is_empty() {
                parsed.truncate(limit);
                return Ok(parsed);
            }
            errors.push("duckduckgo returned no matching results".to_string());
        }
        Err(err) => errors.push(format!("duckduckgo failed: {err}")),
    }

    match search_baidu_with_client(&client, query).await {
        Ok(mut parsed) => {
            parsed.retain(|r| passes_domain_filter(&r.link, &allowed, &blocked));
            parsed.truncate(limit);
            Ok(parsed)
        }
        Err(err) => {
            errors.push(format!("baidu fallback failed: {err}"));
            anyhow::bail!("all search providers failed: {}", errors.join("; "));
        }
    }
}

async fn search_ddg_with_client(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<WebSearchResult>> {
    let mut url = reqwest::Url::parse("https://lite.duckduckgo.com/lite/")?;
    url.query_pairs_mut().append_pair("q", query);

    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("search request failed with status {}", response.status());
    }

    let html = response.text().await?;
    Ok(parse_duckduckgo_lite(&html))
}

async fn search_baidu_with_client(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<WebSearchResult>> {
    let mut url = reqwest::Url::parse("https://www.baidu.com/s")?;
    url.query_pairs_mut().append_pair("wd", query);

    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("search request failed with status {}", response.status());
    }

    let html = response.text().await?;
    Ok(parse_baidu_results(&html))
}

fn parse_duckduckgo_lite(html: &str) -> Vec<WebSearchResult> {
    let mut results = vec![];
    let marker = "<a rel=\"nofollow\" href=\"";
    let mut cursor = 0usize;

    while let Some(link_pos_rel) = html[cursor..].find(marker) {
        let link_pos = cursor + link_pos_rel;
        let href_start = link_pos + marker.len();
        let Some(href_end_rel) = html[href_start..].find('"') else {
            break;
        };
        let href_end = href_start + href_end_rel;
        let raw_href = &html[href_start..href_end];

        let title_start_marker = "class='result-link'>";
        let Some(title_start_rel) = html[href_end..].find(title_start_marker) else {
            cursor = href_end;
            continue;
        };
        let title_start = href_end + title_start_rel + title_start_marker.len();
        let Some(title_end_rel) = html[title_start..].find("</a>") else {
            cursor = title_start;
            continue;
        };
        let title_end = title_start + title_end_rel;

        let next_anchor = html[title_end..]
            .find(marker)
            .map(|i| i + title_end)
            .unwrap_or(html.len());
        let block = &html[title_end..next_anchor];
        let snippet = extract_between(block, "<td class='result-snippet'>", "</td>")
            .map(|s| strip_tags(&s))
            .unwrap_or_default();

        let title = strip_tags(&decode_html(&html[title_start..title_end]));
        let link = normalize_duckduckgo_link(raw_href);
        if !title.is_empty() && !link.is_empty() {
            results.push(WebSearchResult {
                title,
                link,
                snippet: decode_html(&snippet),
            });
        }
        cursor = title_end;
    }

    results
}

fn parse_baidu_results(html: &str) -> Vec<WebSearchResult> {
    let mut results = vec![];
    let marker = "<h3";
    let mut cursor = 0usize;

    while let Some(h3_rel) = html[cursor..].find(marker) {
        let h3_start = cursor + h3_rel;
        let Some(h3_end_rel) = html[h3_start..].find("</h3>") else {
            break;
        };
        let h3_end = h3_start + h3_end_rel + "</h3>".len();
        let block = &html[h3_start..h3_end];

        let Some(a_start_rel) = block.find("<a ") else {
            cursor = h3_end;
            continue;
        };
        let a_start = a_start_rel;
        let href_marker = "href=\"";
        let Some(href_pos_rel) = block[a_start..].find(href_marker) else {
            cursor = h3_end;
            continue;
        };
        let href_start = a_start + href_pos_rel + href_marker.len();
        let Some(href_end_rel) = block[href_start..].find('"') else {
            cursor = h3_end;
            continue;
        };
        let href_end = href_start + href_end_rel;
        let raw_href = &block[href_start..href_end];
        let link = decode_html(raw_href).trim().to_string();

        let Some(title_start_rel) = block[href_end..].find('>') else {
            cursor = h3_end;
            continue;
        };
        let title_start = href_end + title_start_rel + 1;
        let Some(title_end_rel) = block[title_start..].find("</a>") else {
            cursor = h3_end;
            continue;
        };
        let title_end = title_start + title_end_rel;
        let title = strip_tags(&decode_html(&block[title_start..title_end]));

        if title.is_empty() || link.is_empty() {
            cursor = h3_end;
            continue;
        }

        let next_h3 = html[h3_end..]
            .find(marker)
            .map(|i| i + h3_end)
            .unwrap_or(html.len());
        let snippet_block = &html[h3_end..next_h3];
        let snippet = extract_between(snippet_block, "<div class=\"c-abstract\">", "</div>")
            .or_else(|| {
                extract_between(
                    snippet_block,
                    "<span class=\"content-right_8Zs40\">",
                    "</span>",
                )
            })
            .map(|s| strip_tags(&decode_html(&s)))
            .unwrap_or_default();

        results.push(WebSearchResult {
            title,
            link,
            snippet,
        });
        cursor = h3_end;
    }

    results
}

fn normalize_domain_list(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

fn normalize_duckduckgo_link(raw_href: &str) -> String {
    let href = decode_html(raw_href).trim().to_string();
    if href.is_empty() {
        return String::new();
    }
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href
    };
    if let Ok(url) = reqwest::Url::parse(&absolute)
        && let Some(redirect) = url.query_pairs().find_map(|(k, v)| {
            if k == "uddg" {
                Some(v.into_owned())
            } else {
                None
            }
        })
    {
        return redirect;
    }
    absolute
}

#[derive(Debug, Clone)]
struct WebSearchResult {
    title: String,
    link: String,
    snippet: String,
}

fn passes_domain_filter(link: &str, allowed: &[String], blocked: &[String]) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else {
        return false;
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if blocked.iter().any(|d| matches_domain(&host, d)) {
        return false;
    }
    if allowed.is_empty() {
        return true;
    }
    allowed.iter().any(|d| matches_domain(&host, d))
}

fn matches_domain(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}
