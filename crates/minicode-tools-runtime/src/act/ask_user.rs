use async_trait::async_trait;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde_json::Value;
use serde_json::json;

#[derive(Default)]
pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "ask_user"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "向用户提问并暂停当前轮次。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "question":{"type":"string"},
                "options":{"type":"array","items":{"type":"string"},"minItems":1}
            },
            "required":["question"]
        })
    }
    /// 透传问题并要求当前轮等待用户回复。
    async fn run(&self, input: Value) -> ToolResult {
        let question = input
            .get("question")
            .and_then(|x| x.as_str())
            .unwrap_or("请补充信息")
            .to_string();
        let options = input
            .get("options")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec!["继续".to_string(), "取消".to_string()]);
        ToolResult {
            ok: true,
            output: json!({
                "question": question,
                "options": options
            })
            .to_string(),
            background_task: None,
            await_user: true,
        }
    }
}
