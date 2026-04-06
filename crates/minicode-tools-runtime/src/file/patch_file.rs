use crate::file::apply_reviewed_file_change;
use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Default)]
pub struct PatchFileTool;
#[derive(Debug, Deserialize)]
struct Replacement {
    search: String,
    replace: String,
    #[serde(rename = "replaceAll")]
    replace_all: Option<bool>,
}
#[async_trait]
impl Tool for PatchFileTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "patch_file"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "对单文件执行批量替换。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"replacements":{"type":"array","items":{"type":"object","properties":{"search":{"type":"string"},"replace":{"type":"string"},"replaceAll":{"type":"boolean"}},"required":["search","replace"]}}},"required":["path","replacements"]})
    }
    /// 依次应用多组查找替换规则。
    async fn run(&self, input: Value) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let replacements: Vec<Replacement> = match input.get("replacements").cloned() {
            Some(v) => serde_json::from_value(v).unwrap_or_default(),
            None => vec![],
        };
        if path.is_empty() || replacements.is_empty() {
            return ToolResult::err("path/replacements is required");
        }

        let target = match resolve_tool_path(path, "write").await {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        let mut content = match std::fs::read_to_string(&target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        for (idx, rep) in replacements.iter().enumerate() {
            if !content.contains(&rep.search) {
                return ToolResult::err(format!("Replacement {} failed: text not found", idx + 1));
            }
            if rep.replace_all.unwrap_or(false) {
                content = content.replace(&rep.search, &rep.replace);
            } else {
                content = content.replacen(&rep.search, &rep.replace, 1);
            }
        }

        match apply_reviewed_file_change(path, &target, &content).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
