use crate::file::apply_reviewed_file_change;
use crate::resolve_tool_path;
use async_trait::async_trait;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde_json::Value;
use serde_json::json;

#[derive(Default)]
pub struct WriteLikeTool {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}
#[async_trait]
impl Tool for WriteLikeTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        self.name
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        self.description
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    /// 写入或整体替换文件内容（带权限审阅）。
    async fn run(&self, input: Value) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|x| x.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("path is required");
        }

        let target = match resolve_tool_path(path, "write").await {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };

        match apply_reviewed_file_change(path, &target, content).await {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
