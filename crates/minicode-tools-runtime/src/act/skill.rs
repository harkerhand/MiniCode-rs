use async_trait::async_trait;
use minicode_config::runtime_store;
use minicode_skills::load_skill;
use minicode_tool::Tool;
use minicode_tool::ToolResult;
use serde_json::Value;
use serde_json::json;

#[derive(Default)]
pub struct LoadSkillTool;
#[async_trait]
impl Tool for LoadSkillTool {
    /// 返回工具名称。
    fn name(&self) -> &str {
        "load_skill"
    }
    /// 返回工具描述。
    fn description(&self) -> &str {
        "读取某个技能的 SKILL.md 内容。"
    }
    /// 返回输入参数 schema。
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})
    }
    /// 读取指定技能的 SKILL.md 内容。
    async fn run(&self, input: Value) -> ToolResult {
        let name = input.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if name.is_empty() {
            return ToolResult::err("name is required");
        }
        if let Some(skill) = load_skill(&runtime_store().cwd, name) {
            return ToolResult::ok(skill.content);
        }
        ToolResult::err(format!("Skill not found: {name}"))
    }
}
