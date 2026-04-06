use crate::file::apply_reviewed_file_change;
use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde_json::Value;
use serde_json::json;

#[derive(Default)]
pub struct EditFileTool;
#[async_trait]
impl Tool for EditFileTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "edit_file"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "Apply line-by-line edits to files using precise search/replace patterns."
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"search":{"type":"string"},"replace":{"type":"string"},"replaceAll":{"type":"boolean"}},"required":["path","search","replace"]})
    }
    /// 执行单次或全量字符串替换编辑。
    async fn run(&self, input: Value) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let search = input.get("search").and_then(|x| x.as_str()).unwrap_or("");
        let replace = input.get("replace").and_then(|x| x.as_str()).unwrap_or("");
        let replace_all = input
            .get("replaceAll")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if path.is_empty() || search.is_empty() {
            return ToolResult::err("path/search is required");
        }

        let target = match resolve_tool_path(path, "write").await {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let original = match std::fs::read_to_string(&target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        if !original.contains(search) {
            return ToolResult::err(format!("Text not found in {path}"));
        }

        let next = if replace_all {
            original.replace(search, replace)
        } else {
            original.replacen(search, replace, 1)
        };

        match apply_reviewed_file_change(path, &target, &next).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
