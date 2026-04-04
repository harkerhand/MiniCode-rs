use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::permissions::PermissionManager;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: String,
    pub permissions: Option<Arc<PermissionManager>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackgroundTaskResult {
    pub task_id: String,
    pub r#type: String,
    pub command: String,
    pub pid: i32,
    pub status: String,
    pub started_at: i64,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub ok: bool,
    pub output: String,
    pub background_task: Option<BackgroundTaskResult>,
    pub await_user: bool,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
            background_task: None,
            await_user: false,
        }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self {
            ok: false,
            output: output.into(),
            background_task: None,
            await_user: false,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult;
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct McpServerSummary {
    pub name: String,
    pub command: String,
    pub status: String,
    pub tool_count: usize,
    pub error: Option<String>,
    pub protocol: Option<String>,
    pub resource_count: Option<usize>,
    pub prompt_count: Option<usize>,
}

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
    index: HashMap<String, usize>,
    skills: Vec<SkillSummary>,
    mcp_servers: Vec<McpServerSummary>,
    disposer: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
}

impl ToolRegistry {
    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        skills: Vec<SkillSummary>,
        mcp_servers: Vec<McpServerSummary>,
        disposer: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
    ) -> Self {
        let mut index = HashMap::new();
        for (idx, tool) in tools.iter().enumerate() {
            index.insert(tool.name().to_string(), idx);
        }
        Self {
            tools,
            index,
            skills,
            mcp_servers,
            disposer,
        }
    }

    pub fn list(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }

    pub fn get_skills(&self) -> &[SkillSummary] {
        &self.skills
    }

    pub fn get_mcp_servers(&self) -> &[McpServerSummary] {
        &self.mcp_servers
    }

    pub async fn execute(
        &self,
        tool_name: &str,
        input: Value,
        context: &ToolContext,
    ) -> ToolResult {
        let Some(idx) = self.index.get(tool_name) else {
            return ToolResult::err(format!("Unknown tool: {tool_name}"));
        };

        self.tools[*idx].run(input, context).await
    }

    pub async fn dispose(&self) {
        if let Some(disposer) = &self.disposer {
            disposer().await;
        }
    }
}
