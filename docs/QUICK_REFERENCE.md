# MCP 和 Tool 系统快速参考指南

## 📋 文件位置速查

| 文件/模块 | 路径 | 用途 |
|---------|------|------|
| Tool 定义 | `crates/minicode-tool/src/lib.rs` | Tool trait, ToolResult, ToolRegistry |
| 内置工具 | `crates/minicode-tools-runtime/src/lib.rs` | 所有默认工具实现 |
| MCP 支持 | `crates/minicode-mcp/src/lib.rs` | MCP 客户端、工具包装 |
| 配置管理 | `crates/minicode-config/src/lib.rs` | MCP 服务器配置加载 |
| 系统提示 | `crates/minicode-prompt/src/lib.rs` | 系统提示词构建 |

---

## 🔧 快速创建工具 (Template)

### 最小工具模板
```rust
use async_trait::async_trait;
use minicode_tool::{Tool, ToolContext, ToolResult};
use serde_json::{Value, json};

#[derive(Default)]
pub struct MyTool;

#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    
    fn description(&self) -> &str { 
        "Description of what this tool does" 
    }
    
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "param1": { "type": "string" }
            },
            "required": ["param1"]
        })
    }
    
    async fn run(&self, input: Value, context: &ToolContext) -> ToolResult {
        // TODO: Implement tool logic
        ToolResult::ok("output")
    }
}
```

### 工具参数解析快速参考

```rust
// String 参数
let param = input.get("param").and_then(|x| x.as_str()).unwrap_or("default");

// Number 参数
let count = input.get("count").and_then(|x| x.as_u64()).unwrap_or(0);

// Boolean 参数
let force = input.get("force").and_then(|x| x.as_bool()).unwrap_or(false);

// 数组参数
let items: Vec<String> = input.get("items")
    .and_then(|x| x.as_array())
    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
    .unwrap_or_default();

// 对象参数
let nested = input.get("nested").and_then(|x| x.as_object());

// 强类型反序列化
#[derive(Deserialize)]
struct Input { param1: String, param2: Option<i32> }
let parsed: Input = serde_json::from_value(input)?;
```

---

## 📦 Schema 定义速查

### 常见类型定义

```rust
// 字符串
"type": "string"

// 数字
"type": "number"     // 浮点
"type": "integer"    // 整数

// 布尔
"type": "boolean"

// 数组
"type": "array",
"items": { "type": "string" }

// 对象
"type": "object",
"properties": { ... }

// 枚举
"type": "string",
"enum": ["option1", "option2"]

// 数字范围
"type": "number",
"minimum": 0,
"maximum": 100

// 字符串长度
"type": "string",
"minLength": 1,
"maxLength": 100

// 模式匹配
"type": "string",
"pattern": "^[a-z]+$"
```

### 必需字段

```rust
json!({
    "type": "object",
    "properties": { /* ... */ },
    "required": ["param1", "param2"]  // 这些字段必须存在
})
```

---

## ✅ 常见错误处理模式

### 参数不存在
```rust
if input.get("path").is_none() {
    return ToolResult::err("path parameter is required");
}
```

### 参数类型错误
```rust
let path = input.get("path")
    .and_then(|x| x.as_str())
    .ok_or_else(|| "path must be a string")?;
```

### 文件操作错误
```rust
match std::fs::read_to_string(&file) {
    Ok(content) => content,
    Err(err) => return ToolResult::err(format!("Failed to read file: {}", err)),
}
```

### 权限检查错误
```rust
let target = match resolve_tool_path(context, path, "read").await {
    Ok(p) => p,
    Err(err) => return ToolResult::err(err.to_string()),
};
```

### 异步操作超时
```rust
match tokio::time::timeout(Duration::from_secs(30), async_op()).await {
    Ok(Ok(result)) => result,
    Ok(Err(err)) => return ToolResult::err(err.to_string()),
    Err(_) => return ToolResult::err("Operation timed out"),
}
```

---

## 🚀 ToolRegistry 使用

### 创建注册表
```rust
let registry = create_default_tool_registry(&cwd, Some(&runtime_config)).await?;
```

### 获取所有工具列表
```rust
let tools = registry.list();
for tool in tools {
    println!("{}: {}", tool.name(), tool.description());
}
```

### 执行工具
```rust
let context = ToolContext {
    cwd: "/path/to/project".to_string(),
    permissions: None,
};

let result = registry.execute(
    "tool_name",
    json!({"param": "value"}),
    &context,
).await;

if result.ok {
    println!("Success: {}", result.output);
} else {
    println!("Error: {}", result.output);
}
```

### 后台任务处理
```rust
if let Some(bg_task) = result.background_task {
    println!("Task ID: {}", bg_task.task_id);
    println!("PID: {}", bg_task.pid);
}
```

### 等待用户回复
```rust
if result.await_user {
    // 暂停当前流程，等待用户输入
    println!("Question: {}", result.output);
}
```

### 清理资源
```rust
registry.dispose().await;
```

---

## 🔌 MCP 工具集成

### MCP 工具命名约定
```
mcp__<server>__<tool>

示例：
- mcp__filesystem__read
- mcp__github__create_issue
- mcp__sqlite__query
```

### 配置 MCP 服务器

在 `~/.mini-code/mcp.json`:
```json
{
  "mcpServers": {
    "server_name": {
      "command": "npx",
      "args": ["@modelcontextprotocol/server-filesystem", "/home/user"],
      "protocol": "content-length"
    }
  }
}
```

或项目级 `.mcp.json`:
```json
{
  "mcpServers": {
    "local-server": {
      "command": "/path/to/server",
      "env": { "VAR": "value" }
    }
  }
}
```

### 获取 MCP 信息
```rust
// 获取 MCP 服务器摘要
let servers = registry.get_mcp_servers();
for server in servers {
    println!("{}: {} (tools={})", 
        server.name, server.status, server.tool_count);
}
```

### 列出 MCP 资源
```rust
// 使用 list_mcp_resources 工具
let result = registry.execute(
    "list_mcp_resources",
    json!({"server": "filesystem"}),
    &context,
).await;
```

### 读取 MCP 资源
```rust
// 使用 read_mcp_resource 工具
let result = registry.execute(
    "read_mcp_resource",
    json!({
        "server": "filesystem",
        "uri": "file:///home/user/README.md"
    }),
    &context,
).await;
```

---

## ⚙️ 内置工具列表

### 文件操作
- **list_files**: 列出目录内容 (最多 200 条)
- **read_file**: 读取文件内容 (支持分块读取)
- **write_file**: 写入文件内容
- **modify_file**: 修改文件内容 (带 diff 审核)
- **edit_file**: 编辑文件 (搜索替换模式)
- **patch_file**: 批量替换

### 搜索
- **grep_files**: 使用 ripgrep 搜索 (前 100 个匹配)

### 命令执行
- **run_command**: 执行 shell 命令 (支持后台运行)

### 交互
- **ask_user**: 向用户提问并暂停

### 技能
- **load_skill**: 加载技能的 SKILL.md

---

## 🔐 权限和上下文

### ToolContext 结构
```rust
pub struct ToolContext {
    pub cwd: String,  // 当前工作目录
    pub permissions: Option<Arc<PermissionManager>>,  // 权限管理器
}
```

### 权限审核
```rust
// 工具内部可以检查权限
if let Some(perms) = &context.permissions {
    let approval = perms.ensure_command("command", &args, &cwd, None).await?;
    // 如果返回 Err，说明需要用户审批
}
```

### 路径解析
```rust
// 自动处理权限检查和安全性
let target = resolve_tool_path(context, path, "read").await?;
```

---

## 📊 ToolResult 详解

### 返回成功
```rust
ToolResult::ok("Operation completed successfully")
ToolResult {
    ok: true,
    output: "results".to_string(),
    background_task: None,
    await_user: false,
}
```

### 返回错误
```rust
ToolResult::err("Something went wrong")
ToolResult {
    ok: false,
    output: "error message".to_string(),
    background_task: None,
    await_user: false,
}
```

### 返回后台任务
```rust
ToolResult {
    ok: true,
    output: format!("Background command started.\nTASK: {}\nPID: {}", bg.task_id, bg.pid),
    background_task: Some(BackgroundTaskResult { /* ... */ }),
    await_user: false,
}
```

### 请求用户输入
```rust
ToolResult {
    ok: true,
    output: "What is your name?".to_string(),
    background_task: None,
    await_user: true,  // 关键！
}
```

---

## ⏱️ 超时配置

```rust
const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const MCP_INIT_TIMEOUT: Duration = Duration::from_secs(2);
const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(3);
```

---

## 🐛 调试技巧

### 启用 MCP 日志
```rust
use minicode_mcp::set_mcp_logging_enabled;

set_mcp_logging_enabled(true);
```

### 检查工具名称和参数
```rust
// 工具列表
for tool in registry.list() {
    println!("Tool: {}", tool.name());
    println!("  Description: {}", tool.description());
    println!("  Schema: {}", tool.input_schema());
}
```

### 验证 Schema
```rust
use jsonschema::{Validator, Draft};

let schema = json!({"type": "object", "properties": {...}});
match Validator::options().with_draft(Draft::Draft7).build(&schema) {
    Ok(validator) => println!("Schema is valid"),
    Err(err) => println!("Schema error: {}", err),
}
```

---

## 📝 常见问题

### Q: 如何添加新的内置工具？
A: 在 `minicode-tools-runtime/src/lib.rs` 的 `create_default_tool_registry` 函数中：
```rust
let mut tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(MyNewTool),
    // ...
];
```

### Q: MCP 工具何时自动注册？
A: 当调用 `create_default_tool_registry` 时，会自动启动所有配置的 MCP 服务器并注册其工具。

### Q: 如何在运行时动态添加工具？
A: 使用 `ToolRegistry::extend_dynamic_tools()`:
```rust
registry.extend_dynamic_tools(new_tools, mcp_servers, disposer);
```

### Q: 参数验证在哪里进行？
A: 在 `ToolRegistry::execute()` 中，使用 JSON Schema Draft7 验证。

### Q: 如何处理大文件？
A: 使用 `read_file` 的 offset/limit 参数分块读取，检查 TRUNCATED 头。

---

## 📚 扩展阅读

- **完整探索**: 查看 `docs/MCP_TOOL_EXPLORATION.md`
- **代码示例**: 查看 `docs/CODE_EXAMPLES.md`
- **架构文档**: 查看 `docs/ARCHITECTURE.md`
