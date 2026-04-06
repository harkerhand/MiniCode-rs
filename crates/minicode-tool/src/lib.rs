use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use jsonschema::Draft;
use jsonschema::Validator;
use minicode_permissions::PermissionManager;
use minicode_types::{McpServerSummary, SkillSummary};
use serde_json::Value;
use tokio::sync::RwLock;
mod shortcut;
pub use shortcut::*;

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
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
    /// 构造成功结果。
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
            background_task: None,
            await_user: false,
        }
    }

    /// 构造失败结果。
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
    /// 返回工具名称。
    fn name(&self) -> &str;
    /// 返回工具用途描述。
    fn description(&self) -> &str;
    /// 返回工具输入 JSON Schema。
    fn input_schema(&self) -> Value;
    /// 执行工具逻辑。
    async fn run(&self, input: Value) -> ToolResult;
}

enum InputValidator {
    Compiled(Validator),
    CompileError(String),
}

/// 编译工具输入校验器。
fn compile_validator(schema: &Value) -> InputValidator {
    match Validator::options().with_draft(Draft::Draft7).build(schema) {
        Ok(validator) => InputValidator::Compiled(validator),
        Err(err) => InputValidator::CompileError(format!("Invalid tool schema: {err}")),
    }
}

/// 按工具 schema 校验输入参数。
fn validate_tool_input(validator: &InputValidator, input: &Value) -> Result<(), String> {
    match validator {
        InputValidator::Compiled(validator) => {
            if !validator.is_valid(input) {
                let details = validator
                    .iter_errors(input)
                    .take(3)
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>();
                if details.is_empty() {
                    return Err("Invalid input".to_string());
                }
                return Err(format!("Invalid input: {}", details.join("; ")));
            }
            Ok(())
        }
        InputValidator::CompileError(err) => Err(err.clone()),
    }
}

pub struct ToolRegistry {
    state: RwLock<ToolRegistryState>,
}
static TOOL_REGISTRY: OnceLock<ToolRegistry> = OnceLock::new();

/// 初始化全局工具注册表（仅允许一次）。
pub fn init_tool_registry(registry: ToolRegistry) -> Result<()> {
    TOOL_REGISTRY
        .set(registry)
        .map_err(|_| anyhow!("Tool registry already initialized"))
}

/// 获取全局工具注册表。
pub fn get_tool_registry() -> &'static ToolRegistry {
    TOOL_REGISTRY.get().expect("Tool registry not initialized")
}

struct ToolRegistryState {
    tools: Vec<Arc<dyn Tool>>,
    index: HashMap<String, usize>,
    validators: Vec<InputValidator>,
    skills: Vec<SkillSummary>,
    mcp_servers: Vec<McpServerSummary>,
    disposer: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
}

/// 组合两个可选的资源释放回调。
fn combine_disposers(
    left: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
    right: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
) -> Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>> {
    match (left, right) {
        (None, None) => None,
        (Some(one), None) | (None, Some(one)) => Some(one),
        (Some(a), Some(b)) => Some(Arc::new(move || {
            let a = a.clone();
            let b = b.clone();
            Box::pin(async move {
                a().await;
                b().await;
            })
        })),
    }
}

impl ToolRegistry {
    /// 创建工具注册表并建立名称索引。
    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        skills: Vec<SkillSummary>,
        mcp_servers: Vec<McpServerSummary>,
        disposer: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
    ) -> Self {
        let mut index = HashMap::new();
        let mut validators = Vec::with_capacity(tools.len());
        for (idx, tool) in tools.iter().enumerate() {
            index.insert(tool.name().to_string(), idx);
            validators.push(compile_validator(&tool.input_schema()));
        }
        Self {
            state: RwLock::new(ToolRegistryState {
                tools,
                index,
                validators,
                skills,
                mcp_servers,
                disposer,
            }),
        }
    }

    /// 返回当前所有已注册工具。
    pub fn list(&self) -> Vec<Arc<dyn Tool>> {
        self.state
            .try_read()
            .map(|state| state.tools.clone())
            .unwrap_or_default()
    }

    /// 返回已发现技能摘要。
    pub fn get_skills(&self) -> Vec<SkillSummary> {
        self.state
            .try_read()
            .map(|state| state.skills.clone())
            .unwrap_or_default()
    }

    /// 返回 MCP 服务连接摘要。
    pub fn get_mcp_servers(&self) -> Vec<McpServerSummary> {
        self.state
            .try_read()
            .map(|state| state.mcp_servers.clone())
            .unwrap_or_default()
    }

    /// 更新 MCP 服务摘要列表。
    pub fn set_mcp_servers(&self, mcp_servers: Vec<McpServerSummary>) {
        if let Ok(mut state) = self.state.try_write() {
            state.mcp_servers = mcp_servers;
        }
    }

    /// 向注册表追加运行时动态工具。
    pub fn extend_dynamic_tools(
        &self,
        tools: Vec<Arc<dyn Tool>>,
        mcp_servers: Vec<McpServerSummary>,
        disposer: Option<Arc<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>>,
    ) {
        if let Ok(mut state) = self.state.try_write() {
            for tool in tools {
                let name = tool.name().to_string();
                if state.index.contains_key(&name) {
                    continue;
                }
                let idx = state.tools.len();
                state.index.insert(name, idx);
                state
                    .validators
                    .push(compile_validator(&tool.input_schema()));
                state.tools.push(tool);
            }
            state.mcp_servers = mcp_servers;
            state.disposer = combine_disposers(state.disposer.clone(), disposer);
        }
    }

    /// 执行指定工具并返回结果。
    pub async fn execute(&self, tool_name: &str, input: Value) -> ToolResult {
        let state = self.state.read().await;
        let Some(idx) = state.index.get(tool_name) else {
            return ToolResult::err(format!("Unknown tool: {tool_name}"));
        };
        let tool = state.tools[*idx].clone();
        let validation_error = validate_tool_input(&state.validators[*idx], &input).err();
        drop(state);

        if let Some(err) = validation_error {
            return ToolResult::err(err);
        }

        tool.run(input).await
    }

    /// 释放注册表持有的外部资源。
    pub async fn dispose(&self) {
        let state = self.state.read().await;
        let disposer = state.disposer.clone();
        drop(state);
        if let Some(disposer) = disposer {
            disposer().await;
        }
    }
}
