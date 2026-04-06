# MCP 和 Tool 系统代码对比示例

## 1. 简单工具 vs 复杂工具对比

### 简单工具示例 (AskUserTool)
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
    async fn run(&self, input: Value) -> ToolResult {
        let question = input
            .get("question")
            .and_then(|x| x.as_str())
            .unwrap_or("请补充信息")
            .to_string();
        ToolResult {
            ok: true,
            output: question,
            background_task: None,
            await_user: true,  // 设置等待用户标志
        }
    }
}
```

**关键特性:**
- 无状态结构 (#[derive(Default)])
- 单个参数 (question)
- 返回特殊的 await_user 标志

---

### 复杂工具示例 (ReadFileTool)
```rust
#[derive(Default)]
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { 
        "Read UTF-8 text file with optional offset/limit for chunked reading. Check TRUNCATED header." 
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "offset":{"type":"number"},
            "limit":{"type":"number"}
        },"required":["path"]})
    }
    async fn run(&self, input: Value, ) -> ToolResult {
        let path = input.get("path").and_then(|x| x.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("path is required");
        }
        let offset = input.get("offset").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|x| x.as_u64())
            .unwrap_or(8000)
            .min(20_000) as usize;
        
        // 权限检查和路径解析
        let target = match resolve_tool_path( path, "read").await {
            Ok(p) => p,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        
        // 读取文件
        let content = match std::fs::read_to_string(target) {
            Ok(v) => v,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        
        // 分块处理
        let chars = content.chars().collect::<Vec<_>>();
        let total_chars = chars.len();
        let safe_offset = offset.min(total_chars);
        let end = safe_offset.saturating_add(limit).min(total_chars);
        let chunk = chars[safe_offset..end].iter().collect::<String>();
        let truncated = end < total_chars;
        
        // 返回头信息 + 内容
        let header = format!(
            "FILE: {}\nOFFSET: {}\nEND: {}\nTOTAL_CHARS: {}\nTRUNCATED: {}\n\n",
            path,
            safe_offset,
            end,
            total_chars,
            if truncated { format!("yes - call read_file again with offset {}", end) } else { "no".to_string() }
        );
        
        ToolResult::ok(format!("{}{}", header, chunk))
    }
}
```

**关键特性:**
- 多个参数 (path, offset, limit) 带默认值
- 权限检查和路径解析
- 大文件分块处理
- 详细的返回头信息
- 多层错误处理

---

## 2. 参数解析模式对比

### Pattern 1: 直接 and_then
```rust
let path = input.get("path").and_then(|x| x.as_str()).unwrap_or(".");
```

**优点:** 简洁，处理 null/missing 参数
**缺点:** 无错误上下文

---

### Pattern 2: serde_json 反序列化
```rust
#[derive(Debug, Deserialize)]
struct GrepInput {
    pattern: String,
    path: Option<String>,
}

let parsed: GrepInput = match serde_json::from_value(input) {
    Ok(v) => v,
    Err(err) => return ToolResult::err(err.to_string()),
};
```

**优点:** 类型安全，类型检查，清晰的结构
**缺点:** 需要定义结构体

---

### Pattern 3: 手动验证
```rust
if path.is_empty() {
    return ToolResult::err("path is required");
}
```

**优点:** 自定义错误消息
**缺点:** 重复代码多

---

## 3. Schema 定义最佳实践

### 最小 Schema
```rust
fn input_schema(&self) -> Value {
    json!({"type":"object","properties":{"path":{"type":"string"}}})
}
```

**无 required 约束，全部可选**

---

### 带 Required 字段的 Schema
```rust
fn input_schema(&self) -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string" },
            "path": { "type": "string" }
        },
        "required": ["pattern"]
    })
}
```

**pattern 是必需的，path 可选**

---

### 复杂 Schema
```rust
fn input_schema(&self) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "replacements": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "search": { "type": "string" },
                        "replace": { "type": "string" },
                        "replaceAll": { "type": "boolean" }
                    },
                    "required": ["search", "replace"]
                }
            }
        },
        "required": ["path", "replacements"]
    })
}
```

**嵌套对象数组，内部有 required 约束**

---

## 4. MCP 工具包装流程

### 步骤 1: 描述符结构
```rust
#[derive(Debug, Clone, Deserialize)]
struct McpToolDescriptor {
    name: String,
    description: Option<String>,
    #[serde(rename = "inputSchema")]
    input_schema: Option<Value>,
}
```

**从 MCP 服务器接收的工具元数据**

---

### 步骤 2: 动态工具类
```rust
struct McpDynamicTool {
    wrapped_name: String,  // "mcp__server__tool"
    description: String,
    input_schema: Value,
    tool_name: String,  // 原始名称
    client: Arc<Mutex<StdioMcpClient>>,
}
```

**包装后的工具，持有 MCP 客户端连接**

---

### 步骤 3: 工具实现
```rust
#[async_trait]
impl Tool for McpDynamicTool {
    fn name(&self) -> &str { &self.wrapped_name }
    fn description(&self) -> &str { &self.description }
    fn input_schema(&self) -> Value { self.input_schema.clone() }
    
    async fn run(&self, input: Value) -> ToolResult {
        let mut client = self.client.lock().await;  // 获取互斥锁
        match client.call_tool(&self.tool_name, input) {
            Ok(result) => result,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}
```

**通过互斥锁访问共享的 MCP 客户端**

---

### 步骤 4: MCP 客户端调用
```rust
fn call_tool(&mut self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
    let result = self.request(
        "tools/call",
        json!({
            "name": name,
            "arguments": input,
        }),
    )?;
    Ok(format_tool_result(result))
}
```

**发送 JSON-RPC 请求到 MCP 服务器**

---

## 5. ToolRegistry 执行流程

### 执行过程源代码
```rust
pub async fn execute(
    &self,
    tool_name: &str,
    input: Value,
    ,
) -> ToolResult {
    // 第 1 步：获取读锁
    let state = self.state.read().await;
    
    // 第 2 步：查找工具
    let Some(idx) = state.index.get(tool_name) else {
        return ToolResult::err(format!("Unknown tool: {tool_name}"));
    };
    
    // 第 3 步：提前获取验证器 (在释放锁之前)
    let tool = state.tools[*idx].clone();
    let validation_error = validate_tool_input(&state.validators[*idx], &input).err();
    
    // 第 4 步：释放读锁（重要！允许其他并发访问）
    drop(state);
    
    // 第 5 步：处理验证错误
    if let Some(err) = validation_error {
        return ToolResult::err(err);
    }
    
    // 第 6 步：执行工具
    tool.run(input, context).await
}
```

**关键点:**
- 验证在锁内进行，但在工具执行前释放
- 工具执行不持有锁，允许并发
- 克隆工具 Arc，释放锁后执行

---

### 动态工具注册
```rust
pub fn extend_dynamic_tools(
    &self,
    tools: Vec<Arc<dyn Tool>>,
    mcp_servers: Vec<McpServerSummary>,
    disposer: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,
) {
    if let Ok(mut state) = self.state.try_write() {
        for tool in tools {
            let name = tool.name().to_string();
            if state.index.contains_key(&name) {
                continue;  // 跳过重复的工具
            }
            let idx = state.tools.len();
            state.index.insert(name, idx);
            state.validators.push(compile_validator(&tool.input_schema()));
            state.tools.push(tool);
        }
        state.mcp_servers = mcp_servers;
        state.disposer = combine_disposers(state.disposer.clone(), disposer);
    }
}
```

**支持 MCP 工具的运行时动态注册**

---

## 6. 错误处理模式

### 模式 1: 参数验证错误
```rust
if path.is_empty() {
    return ToolResult::err("path is required");
}
```

---

### 模式 2: 权限或解析错误
```rust
let target = match resolve_tool_path( path, "read").await {
    Ok(p) => p,
    Err(err) => return ToolResult::err(err.to_string()),
};
```

---

### 模式 3: 操作失败
```rust
let content = match std::fs::read_to_string(target) {
    Ok(v) => v,
    Err(err) => return ToolResult::err(err.to_string()),
};
```

---

### 模式 4: MCP 通信错误
```rust
match client.call_tool(&self.tool_name, input) {
    Ok(result) => result,
    Err(err) => ToolResult::err(err.to_string()),
}
```

**统一的错误处理：所有错误最终转换为 ToolResult::err()**

---

## 7. JSON-RPC 协议细节

### 请求格式
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "tool_name",
    "arguments": { "param": "value" }
  }
}
```

---

### 成功响应
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      { "type": "text", "text": "output content" }
    ],
    "isError": false
  }
}
```

---

### 错误响应
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32600,
    "message": "Invalid Request"
  }
}
```

---

## 8. 协议交换时序图

```
Client                    MCP Server
  |                           |
  |--- initialize RPC ------->|
  |<--- initialize result ----|
  |                           |
  |--- notifications/initialized -->|
  |                           |
  |--- tools/list RPC ------->|
  |<--- tools list result ----|
  |                           |
  |--- tools/call RPC ------->|
  |<--- tool result ---------|
  |                           |
  (... more requests ...)
```

---

## 9. 超时处理示例

### 带超时的请求
```rust
pub fn request_with_timeout(
    &mut self,
    method: &str,
    params: Value,
    timeout: Duration,
) -> anyhow::Result<Value> {
    let id = self.next_id;
    self.next_id += 1;
    
    // 发送请求
    self.send(&msg)?;
    
    // 设置超时截止时间
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(anyhow::anyhow!(
                "MCP {} request timed out for {}",
                self.server_name,
                method
            ));
        }
        
        // 计算剩余超时
        let remaining = deadline.saturating_duration_since(now);
        
        // 尝试接收响应
        let reply = match self.responses.recv_timeout(remaining) {
            Ok(Ok(message)) => message,
            Ok(Err(err)) => return Err(err),
            Err(RecvTimeoutError::Timeout) => {
                return Err(anyhow::anyhow!("MCP {} request timed out for {}", self.server_name, method));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(anyhow::anyhow!("MCP {} reader disconnected", self.server_name));
            }
        };
        
        // 检查响应 ID
        if reply.id != Some(id) {
            continue;  // 这是对其他请求的响应，继续等待
        }
        
        // 检查错误
        if let Some(err) = reply.error {
            return Err(anyhow::anyhow!(
                "MCP {} error {}: {}",
                self.server_name,
                err.code,
                err.message
            ));
        }
        
        return Ok(reply.result.unwrap_or(Value::Null));
    }
}
```

**关键点:**
- 计算绝对截止时间
- 不断检查是否超时
- 为每次 recv 传递剩余时间
- 处理多个响应到达的情况（ID 匹配）

---

## 10. MCP 启动流程详解

### 启动序列
```rust
pub async fn create_mcp_backed_tools(
    cwd: &Path,
    mcp_servers: &HashMap<String, McpServerConfig>,
) -> McpBundle {
    // Step 1: 创建异步任务队列
    let mut pending = FuturesUnordered::new();
    
    // Step 2: 为每个 MCP 服务器添加启动任务
    for (server_name, config) in mcp_servers {
        pending.push(async move {
            // Step 2a: 在线程池中阻塞式启动
            let join = tokio::task::spawn_blocking({
                move || StdioMcpClient::start(&server_name, &config, &cwd)
            });
            
            // Step 2b: 应用超时控制
            tokio::time::timeout(MCP_STARTUP_TIMEOUT, join).await
        });
    }
    
    // Step 3: 并发收集结果
    while let Some(outcome) = pending.next().await {
        match outcome {
            ConnectOutcome::Success(success) => {
                // 构建 McpDynamicTool
                for descriptor in &success.tool_descriptors {
                    tools.push(Arc::new(McpDynamicTool {
                        wrapped_name: format!(
                            "mcp__{}__{}",
                            sanitize_segment(&server_name),
                            sanitize_segment(&descriptor.name)
                        ),
                        description: descriptor.description.clone().unwrap_or_else(|| {
                            format!(
                                "Call MCP tool {} from server {}.",
                                descriptor.name, server_name
                            )
                        }),
                        input_schema: descriptor.input_schema.clone().unwrap_or_else(
                            || json!({"type":"object","additionalProperties":true}),
                        ),
                        tool_name: descriptor.name.clone(),
                        client: client.clone(),
                    }));
                }
            }
            ConnectOutcome::Failure { server_name, config, error } => {
                // 记录错误
                servers.push(McpServerSummary {
                    status: "error".to_string(),
                    error: Some(error),
                    ..
                });
            }
        }
    }
    
    // Step 4: 创建清理回调
    let disposer = if closers.is_empty() {
        None
    } else {
        Some(Arc::new(move || {
            // 关闭所有 MCP 连接
        }))
    };
    
    McpBundle { tools, servers, disposer }
}
```

**关键设计:**
- 使用 FuturesUnordered 并发启动
- spawn_blocking 避免阻塞 async 线程
- 超时控制防止无限等待
- 清理回调确保资源释放

