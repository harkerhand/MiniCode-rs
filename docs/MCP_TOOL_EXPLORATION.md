# MCP 和 Tool 系统详细探索指南

## 1. 项目结构概览

### 核心 Crates 关系
```
minicode-tool (基础)
    ↓
minicode-tools-runtime (运行时实现)
    ↓
minicode-mcp (MCP 协议实现)
    ↑
minicode-config (配置管理)
```

### Crates 分布
- **minicode-tool**: 工具接口定义 (Tool trait 和 ToolResult)
- **minicode-tools-runtime**: 工具运行时实现 (内置工具、注册表)
- **minicode-mcp**: MCP 通信和工具包装
- **minicode-config**: 配置加载和 MCP 服务器配置
- **minicode-prompt**: 系统提示词生成

---

## 2. Tool Trait 定义 (minicode-tool/src/lib.rs)

### 核心接口
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    /// 返回工具名称 (string)
    fn name(&self) -> &str;
    
    /// 返回工具用途描述 (string)
    fn description(&self) -> &str;
    
    /// 返回工具输入 JSON Schema (Value)
    fn input_schema(&self) -> Value;
    
    /// 执行工具逻辑 (async)
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult;
}
```

### ToolContext 上下文
```rust
pub struct ToolContext {
    pub cwd: String,  // 当前工作目录
    pub permissions: Option<Arc<PermissionManager>>,  // 权限管理器
}
```

### ToolResult 返回结构
```rust
pub struct ToolResult {
    pub ok: bool,  // 执行是否成功
    pub output: String,  // 输出内容
    pub background_task: Option<BackgroundTaskResult>,  // 后台任务信息
    pub await_user: bool,  // 是否等待用户回复
}

// 便捷方法
impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self { ... }
    pub fn err(output: impl Into<String>) -> Self { ... }
}
```

---

## 3. input_schema 和参数验证

### Schema 编译器
```rust
enum InputValidator {
    Compiled(Validator),  // jsonschema 库编译的验证器
    CompileError(String),  // 编译错误
}

fn compile_validator(schema: &Value) -> InputValidator {
    // 使用 jsonschema 库的 Draft7 标准
    match Validator::options().with_draft(Draft::Draft7).build(schema) {
        Ok(validator) => InputValidator::Compiled(validator),
        Err(err) => InputValidator::CompileError(format!("Invalid tool schema: {err}")),
    }
}
```

### 验证过程
```rust
fn validate_tool_input(validator: &InputValidator, input: &Value) -> Result<(), String> {
    match validator {
        InputValidator::Compiled(validator) => {
            if !validator.is_valid(input) {
                let details = validator
                    .iter_errors(input)
                    .take(3)  // 取前 3 个错误
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
```

### Schema 示例 (来自 tools-runtime)
```json
// ListFilesTool 的 schema
{
  "type": "object",
  "properties": {
    "path": { "type": "string" }
  }
}

// ReadFileTool 的 schema
{
  "type": "object",
  "properties": {
    "path": { "type": "string" },
    "offset": { "type": "number" },
    "limit": { "type": "number" }
  },
  "required": ["path"]
}

// GrepFilesTool 的 schema
{
  "type": "object",
  "properties": {
    "pattern": { "type": "string" },
    "path": { "type": "string" }
  },
  "required": ["pattern"]
}
```

---

## 4. ToolRegistry 工作原理

### 注册表结构
```rust
pub struct ToolRegistry {
    state: RwLock<ToolRegistryState>,
}

struct ToolRegistryState {
    tools: Vec<Arc<dyn Tool>>,  // 所有工具
    index: HashMap<String, usize>,  // 名称→索引的映射
    validators: Vec<InputValidator>,  // 对应的验证器
    skills: Vec<SkillSummary>,  // 发现的技能
    mcp_servers: Vec<McpServerSummary>,  // MCP 服务器摘要
    disposer: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,  // 清理回调
}
```

### 关键方法
```rust
// 创建新注册表
pub fn new(
    tools: Vec<Arc<dyn Tool>>,
    skills: Vec<SkillSummary>,
    mcp_servers: Vec<McpServerSummary>,
    disposer: Option<...>,
) -> Self

// 执行工具
pub async fn execute(
    &self,
    tool_name: &str,
    input: Value,
    context: &ToolContext,
) -> ToolResult {
    // 1. 查找工具
    let Some(idx) = state.index.get(tool_name) else {
        return ToolResult::err(format!("Unknown tool: {tool_name}"));
    };
    
    // 2. 验证输入 (在释放 state 之前)
    let tool = state.tools[*idx].clone();
    let validation_error = validate_tool_input(&state.validators[*idx], &input).err();
    drop(state);  // 重要：释放读锁
    
    // 3. 处理验证错误
    if let Some(err) = validation_error {
        return ToolResult::err(err);
    }
    
    // 4. 执行工具
    tool.run(input, context).await
}

// 动态追加工具
pub fn extend_dynamic_tools(
    &self,
    tools: Vec<Arc<dyn Tool>>,
    mcp_servers: Vec<McpServerSummary>,
    disposer: Option<...>,
)

// 清理资源
pub async fn dispose(&self)
```

---

## 5. 内置工具实现示例 (minicode-tools-runtime)

### AskUserTool - 询问用户
```rust
#[derive(Default)]
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str { "ask_user" }
    fn description(&self) -> &str { "向用户提问并暂停当前轮次。" }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"question":{"type":"string"}},"required":["question"]})
    }
    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let question = input.get("question").and_then(|x| x.as_str()).unwrap_or("请补充信息").to_string();
        ToolResult {
            ok: true,
            output: question,
            background_task: None,
            await_user: true,  // 关键字段！
        }
    }
}
```

### ListFilesTool - 列出文件
```rust
#[derive(Default)]
pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str { "list_files" }
    fn description(&self) -> &str { "列出目录内容（最多200条）。" }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or(".");
        let target = match resolve_tool_path(context, path, "list").await {
            Ok(p) => p,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        
        // ... 读取目录，收集条目 ...
        
        ToolResult::ok(if lines.is_empty() {
            "(empty)".to_string()
        } else {
            lines.join("\n")
        })
    }
}
```

### EditFileTool - 编辑文件
```rust
#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str { "edit_file" }
    fn description(&self) -> &str { "Apply line-by-line edits to files using precise search/replace patterns." }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "search":{"type":"string"},
            "replace":{"type":"string"},
            "replaceAll":{"type":"boolean"}
        },"required":["path","search","replace"]})
    }
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let search = input.get("search").and_then(|x| x.as_str()).unwrap_or("");
        let replace = input.get("replace").and_then(|x| x.as_str()).unwrap_or("");
        let replace_all = input.get("replaceAll").and_then(|x| x.as_bool()).unwrap_or(false);
        
        // 权限审核、读取、替换、写回...
        apply_reviewed_file_change(context, path, &target, &next).await
    }
}
```

---

## 6. MCP 集成工作流

### MCP 工具包装 (McpDynamicTool)
```rust
struct McpDynamicTool {
    wrapped_name: String,  // 如 "mcp__server__tool"
    description: String,
    input_schema: Value,
    tool_name: String,  // 原始 MCP 工具名
    client: Arc<Mutex<StdioMcpClient>>,  // MCP 服务连接
}

#[async_trait]
impl Tool for McpDynamicTool {
    fn name(&self) -> &str { &self.wrapped_name }
    fn description(&self) -> &str { &self.description }
    fn input_schema(&self) -> Value { self.input_schema.clone() }
    
    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        match client.call_tool(&self.tool_name, input) {
            Ok(result) => result,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
```

### MCP 启动流程 (StdioMcpClient)
```rust
// 1. 启动子进程
fn start(server_name: &str, config: &McpServerConfig, cwd: &Path) -> anyhow::Result<Self>

// 2. 选择协议 (content-length 或 newline-json)
// 3. 初始化握手 (initialize RPC)
// 4. 发送初始化通知 (notifications/initialized)
// 5. 列出工具、资源、提示

// JSON-RPC 通信
fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value>
fn request_with_timeout(&mut self, method: &str, params: Value, timeout: Duration) -> anyhow::Result<Value>

// 后台读取线程
fn spawn_reader_loop(server_name: String, stdout: BufReader<ChildStdout>, protocol: JsonRpcProtocol, tx: Sender<...>) -> JoinHandle<()>

// 列表操作
fn list_tools(&mut self) -> anyhow::Result<Vec<McpToolDescriptor>>
fn list_resources(&mut self) -> anyhow::Result<Vec<McpResourceDescriptor>>
fn list_prompts(&mut self) -> anyhow::Result<Vec<McpPromptDescriptor>>

// 工具执行
fn call_tool(&mut self, name: &str, input: Value) -> anyhow::Result<ToolResult>
```

### MCP 结果格式化
```rust
fn format_tool_result(result: Value) -> ToolResult {
    let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
    
    let mut parts = vec![];
    
    // 1. 提取 content 数组中的文本块
    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        for block in content {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                parts.push(block.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string());
            } else {
                parts.push(serde_json::to_string_pretty(block)?);
            }
        }
    }
    
    // 2. 提取结构化内容
    if let Some(structured) = result.get("structuredContent") {
        parts.push(format!("STRUCTURED_CONTENT:\n{}", serde_json::to_string_pretty(structured)?));
    }
    
    // 3. 没有内容则返回全部 JSON
    if parts.is_empty() {
        parts.push(serde_json::to_string_pretty(&result)?);
    }
    
    ToolResult {
        ok: !is_error,
        output: parts.join("\n\n"),
        background_task: None,
        await_user: false,
    }
}
```

### MCP 并发启动
```rust
pub async fn create_mcp_backed_tools(
    cwd: &Path,
    mcp_servers: &HashMap<String, McpServerConfig>,
) -> McpBundle {
    let mut pending = FuturesUnordered::new();
    
    // 为每个 MCP 服务器创建启动任务
    for (server_name, config) in mcp_servers {
        pending.push(async move {
            let join = tokio::task::spawn_blocking({
                // 阻塞式启动 (subprocess spawn 不能异步)
                move || StdioMcpClient::start(&server_name, &config, &cwd)
            });
            
            // 应用超时控制
            tokio::time::timeout(MCP_STARTUP_TIMEOUT, join).await
        });
    }
    
    // 并发收集结果
    while let Some(outcome) = pending.next().await {
        // 处理每个启动结果...
    }
}
```

---

## 7. 配置体系

### MCP 服务器配置 (McpServerConfig)
```rust
pub struct McpServerConfig {
    pub command: String,  // 启动命令
    pub args: Option<Vec<String>>,  // 命令参数
    pub env: Option<HashMap<String, serde_json::Value>>,  // 环境变量
    pub cwd: Option<String>,  // 工作目录
    pub enabled: Option<bool>,  // 是否启用
    pub protocol: Option<String>,  // "content-length" 或 "newline-json"
}
```

### 配置加载优先级 (minicode-config)
```rust
fn load_effective_settings(cwd: impl AsRef<Path>) -> Result<MiniCodeSettings> {
    // 优先级从低到高：
    1. ~/.claude/settings.json (Claude 默认配置)
    2. ~/.mini-code/mcp.json (全局 MCP 配置)
    3. .mcp.json (项目级 MCP 配置)
    4. ~/.mini-code/settings.json (全局小 code 配置)
}
```

### 配置文件位置
- 全局设置: `~/.mini-code/settings.json`
- 全局 MCP: `~/.mini-code/mcp.json`
- 项目 MCP: `.mcp.json`
- Claude 兼容: `~/.claude/settings.json`

---

## 8. 工具创建工作流总结

### Step 1: 定义结构体
```rust
#[derive(Default)]  // 或其他字段
pub struct MyTool {
    // 字段定义...
}
```

### Step 2: 实现 Tool Trait
```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    
    fn description(&self) -> &str { "描述工具功能" }
    
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "param1": { "type": "string" },
                "param2": { "type": "number" }
            },
            "required": ["param1"]
        })
    }
    
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        // 解析输入
        let param1 = input.get("param1").and_then(|x| x.as_str()).unwrap_or("");
        
        // 执行逻辑
        match do_something(param1, context) {
            Ok(output) => ToolResult::ok(output),
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
```

### Step 3: 注册工具
```rust
// 在 create_default_tool_registry 中
let tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(MyTool::default()),
    // ... 其他工具 ...
];

let registry = ToolRegistry::new(tools, skills, mcp_servers, disposer);
```

### Step 4: MCP 工具自动加载
```rust
// MCP 工具由 create_mcp_backed_tools 自动：
// 1. 启动子进程
// 2. 初始化握手
// 3. 列表查询
// 4. 自动包装为 McpDynamicTool
// 5. 注册到 ToolRegistry
```

---

## 9. 错误处理模式

### 参数验证错误
```rust
// ToolRegistry.execute() 中自动处理
if let Some(err) = validation_error {
    return ToolResult::err(err);  // 验证失败，立即返回
}
```

### 运行时错误
```rust
// 工具内部处理
async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
    match resolve_tool_path(context, path, "read").await {
        Ok(p) => p,
        Err(err) => return ToolResult::err(err.to_string()),  // 转换为 ToolResult 错误
    };
}
```

### MCP 通信错误
```rust
// StdioMcpClient 中处理超时和协议错误
let deadline = Instant::now() + timeout;
loop {
    let remaining = deadline.saturating_duration_since(now);
    let reply = match self.responses.recv_timeout(remaining) {
        Ok(Ok(message)) => message,
        Ok(Err(err)) => return Err(err),  // 读线程错误
        Err(RecvTimeoutError::Timeout) => return Err(...),  // 超时
        Err(RecvTimeoutError::Disconnected) => return Err(...),  // 连接断开
    };
}
```

---

## 10. 关键超时设置

```rust
const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(45);  // 启动子进程
const MCP_INIT_TIMEOUT: Duration = Duration::from_secs(2);  // 初始化握手
const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);  // 普通请求
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(3);  // 列表查询
```

---

## 11. 后台任务支持

```rust
pub struct BackgroundTaskResult {
    pub task_id: String,
    pub r#type: String,
    pub command: String,
    pub pid: i32,
    pub status: String,
    pub started_at: i64,
}

// 在 ToolResult 中返回
pub struct ToolResult {
    pub background_task: Option<BackgroundTaskResult>,  // 后台任务信息
    // ...
}

// 示例 (RunCommandTool 中)
if use_shell && background {
    let bg = register_background_shell_task(&command_text, pid, cwd);
    ToolResult {
        ok: true,
        output: format!("Background command started.\nTASK: {}\nPID: {}", bg.task_id, bg.pid),
        background_task: Some(bg),
        await_user: false,
    }
}
```

