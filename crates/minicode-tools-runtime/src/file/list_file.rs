use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde_json::Value;
use serde_json::json;
#[derive(Default)]
pub struct ListFilesTool;
#[async_trait]
impl Tool for ListFilesTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "list_files"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "列出目录内容（最多200条）。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
    /// 列出目标目录中的文件和子目录。
    async fn run(&self, input: Value) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or(".");
        let target = match resolve_tool_path(path, "list").await {
            Ok(p) => p,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let entries = match std::fs::read_dir(target) {
            Ok(x) => x,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let mut lines = vec![];
        for entry in entries.take(200).flatten() {
            let prefix = if entry.file_type().map(|f| f.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            lines.push(format!(
                "{} {}",
                prefix,
                entry.file_name().to_string_lossy()
            ));
        }
        ToolResult::ok(if lines.is_empty() {
            "(empty)".to_string()
        } else {
            lines.join("\n")
        })
    }
}
