use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde_json::Value;
use serde_json::json;
#[derive(Default)]
pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "read_file"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "Read UTF-8 text file with optional offset/limit for chunked reading. Check TRUNCATED header."
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["path"]})
    }
    /// 分块读取 UTF-8 文件并带上截断头信息。
    async fn run(&self, input: Value) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("path is required");
        }
        let offset = input.get("offset").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|x| x.as_u64())
            .unwrap_or(8000)
            .min(20_000) as usize;

        let target = match resolve_tool_path(path, "read").await {
            Ok(p) => p,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let content = match std::fs::read_to_string(target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let chars = content.chars().collect::<Vec<_>>();
        let total_chars = chars.len();
        let safe_offset = offset.min(total_chars);
        let end = safe_offset.saturating_add(limit).min(total_chars);
        let chunk = chars[safe_offset..end].iter().collect::<String>();
        let truncated = end < total_chars;
        let header = format!(
            "FILE: {}\nOFFSET: {}\nEND: {}\nTOTAL_CHARS: {}\nTRUNCATED: {}\n\n",
            path,
            safe_offset,
            end,
            total_chars,
            if truncated {
                format!("yes - call read_file again with offset {}", end)
            } else {
                "no".to_string()
            }
        );

        ToolResult::ok(format!("{}{}", header, chunk))
    }
}
