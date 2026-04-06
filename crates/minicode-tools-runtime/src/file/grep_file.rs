use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_config::runtime_store;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::process::Command;

#[derive(Default)]
pub struct GrepFilesTool;
#[derive(Debug, Deserialize)]
struct GrepInput {
    pattern: String,
    path: Option<String>,
}
#[async_trait]
impl Tool for GrepFilesTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "grep_files"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "Search text using ripgrep, with results limited to first 100 matches for performance."
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"]})
    }
    /// 使用 `rg` 搜索文本并返回匹配结果。
    async fn run(&self, input: Value) -> ToolResult {
        let parsed: GrepInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let mut args = vec!["-n".to_string(), "--no-heading".to_string(), parsed.pattern];
        if let Some(path) = parsed.path {
            let p = match resolve_tool_path(&path, "search").await {
                Ok(v) => v,
                Err(err) => return ToolResult::err(err.to_string()),
            };
            args.push(p.to_string_lossy().to_string());
        } else {
            args.push(".".to_string());
        }

        match Command::new("rg")
            .args(args)
            .current_dir(runtime_store().cwd.clone())
            .output()
            .await
        {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let text = if stdout.is_empty() && stderr.is_empty() {
                    "(no matches)".to_string()
                } else if stdout.is_empty() {
                    stderr
                } else if stderr.is_empty() {
                    stdout
                } else {
                    format!("{}\n{}", stdout, stderr)
                };

                // Check if output might be truncated and add indicator
                let result_lines_count = text.lines().count();
                let final_text = if result_lines_count >= 100 {
                    format!(
                        "{}\n\n[Results limited to first 100 matches. Refine your search pattern for more specific results.]",
                        text
                    )
                } else {
                    text
                };
                ToolResult::ok(final_text)
            }
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
