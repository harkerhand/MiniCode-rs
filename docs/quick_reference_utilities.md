# Rust HTTP/网络功能快速参考

## 📌 核心概念速查表

### HTTP 客户端使用

| 功能 | 代码 | 说明 |
|------|------|------|
| 创建客户端 | `let client = reqwest::Client::new();` | 全局共享，支持连接复用 |
| GET 请求 | `client.get(url).send().await?` | 发送 GET 请求 |
| POST 请求 | `client.post(url).json(&body).send().await?` | 发送 JSON POST 请求 |
| 自定义 Header | `client.post(url).headers(headers)` | 添加自定义 HTTP Header |
| 获取响应体 | `resp.text().await?` 或 `resp.json().await?` | 文本或 JSON 格式 |
| 检查状态码 | `resp.status().is_success()` | 检查 HTTP 状态 |

### 异步编程

| 功能 | 代码 | 说明 |
|------|------|------|
| 异步函数 | `async fn foo() { }` | 定义异步函数 |
| 等待异步 | `await?` | 等待异步操作完成 |
| 异步睡眠 | `tokio::time::sleep(Duration).await` | 异步等待 |
| 异步 Trait | `#[async_trait] pub trait Foo { async fn bar(); }` | 定义异步方法的 Trait |
| 运行异步 | `#[tokio::main] async fn main()` | 入口函数宏 |

### 错误处理

| 功能 | 代码 | 说明 |
|------|------|------|
| 结果类型 | `Result<T, E>` 或 `anyhow::Result<T>` | 错误处理类型 |
| 错误传播 | `value?` | 错误传播操作符 |
| 创建错误 | `anyhow!("message")` 或 `Err(msg)` | 创建错误 |
| 链式映射 | `.map_err(\|e\| ...)` | 转换错误类型 |
| 解包 | `.unwrap_or_default()` 或 `.ok()` | 安全地提取值 |

### 重试机制

| 功能 | 代码 | 说明 |
|------|------|------|
| 判断重试 | `status == 429 \|\| (500..600).contains(&status)` | 429 和 5xx 需要重试 |
| 指数退避 | `500ms * 2^attempt` | 延迟增长 |
| 随机抖动 | `base * (1 + random(0..0.25))` | 避免雷群 |
| 最大延迟 | `.min(8000ms)` | 防止过长等待 |
| 环境变量 | `std::env::var("RETRY_LIMIT")` | 配置重试次数 |

---

## 🔍 文件位置速查

```
crates/
├── minicode-agent-core/
│   └── src/anthropic_adapter.rs        ← 🌟 HTTP 请求实现
│       ├── AnthropicModelAdapter
│       ├── 重试逻辑
│       └── Retry-After 解析
│
├── minicode-tool/
│   └── src/lib.rs                       ← 🌟 工具框架
│       ├── Tool trait
│       ├── ToolRegistry
│       ├── ToolResult
│       └── 输入验证
│
├── minicode-mcp/
│   └── src/lib.rs                       ← MCP 工具实现
│       ├── StdioMcpClient
│       ├── JSON-RPC
│       └── 子进程通信
│
└── minicode-types/
    └── src/lib.rs                       ← 类型定义
        ├── ChatMessage
        └── AgentStep
```

---

## 💡 常见模式

### 模式 1：简单的 HTTP 请求

```rust
let client = reqwest::Client::new();
let response = client.get("https://api.example.com/data").send().await?;
let data: MyStruct = response.json().await?;
```

### 模式 2：带 Header 的 POST 请求

```rust
let mut headers = reqwest::header::HeaderMap::new();
headers.insert("Authorization", HeaderValue::from_str("Bearer token")?);

let response = client
    .post(url)
    .headers(headers)
    .json(&request_body)
    .send()
    .await?;
```

### 模式 3：重试循环

```rust
for attempt in 0..=max_retries {
    let resp = client.post(url).send().await?;
    
    if resp.status().is_success() {
        return Ok(resp.json().await?);
    }
    
    if should_retry(resp.status()) && attempt < max_retries {
        let delay = calculate_retry_delay(attempt);
        tokio::time::sleep(delay).await;
        continue;
    }
    
    return Err(anyhow!("Request failed"));
}
```

### 模式 4：实现工具

```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "..." }
    fn input_schema(&self) -> Value { json!({...}) }
    
    async fn run(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        // 验证输入
        let param = input.get("param").and_then(|v| v.as_str())?;
        
        // 执行逻辑
        match do_something(param).await {
            Ok(result) => ToolResult::ok(result),
            Err(e) => ToolResult::err(e.to_string()),
        }
    }
}
```

---

## 🚀 性能优化建议

### ✅ 好的做法

- ✓ 重用 `reqwest::Client` 实例
- ✓ 使用异步编程处理 I/O 密集的操作
- ✓ 实现指数退避重试
- ✓ 尊重服务器的 `Retry-After` header
- ✓ 使用连接池（reqwest 自动）

### ❌ 避免的做法

- ✗ 为每个请求创建新的 `Client`
- ✗ 使用同步网络操作阻塞异步 runtime
- ✗ 立即重试失败的请求
- ✗ 过长的重试延迟（导致超时）
- ✗ 阻塞式的 `sleep` 而不是 `tokio::time::sleep`

---

## 📊 重试延迟参考

| 尝试次数 | 基础延迟 | 加抖动后 | 总耗时 |
|--------|---------|---------|--------|
| 1 | 500ms | 500-625ms | ~550ms |
| 2 | 1000ms | 1000-1250ms | ~1.1s |
| 3 | 2000ms | 2000-2500ms | ~3.2s |
| 4 | 4000ms | 4000-5000ms | ~7.2s |

*注：实际值受 Retry-After header 和系统时钟影响*

---

## 🔧 常用依赖版本（Minicode-rs）

```toml
[workspace.dependencies]
reqwest = { version = "0.13.2", features = ["json", "rustls"] }
tokio = { version = "1.51.0", features = ["time", "rt-multi-thread"] }
serde_json = "1.0.149"
anyhow = "1.0.102"
async-trait = "0.1.89"
httpdate = "1.0.3"              # 解析 Retry-After header
jsonschema = "0.45.0"           # JSON Schema 验证
```

---

## 🎯 关键代码片段

### 获取重试限制
```rust
fn get_retry_limit() -> usize {
    std::env::var("MINI_CODE_MAX_RETRIES")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(4)
}
```

### 检查是否应该重试
```rust
fn should_retry(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}
```

### 解析 Retry-After
```rust
fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    let raw = headers.get("retry-after")?.to_str().ok()?;
    // 尝试解析为秒数
    raw.parse::<u64>().ok().map(|s| s * 1000)
}
```

### 计算退避延迟
```rust
fn retry_delay_ms(attempt: usize) -> u64 {
    let base = 500 * (2u64.pow(attempt as u32 - 1));
    let jitter = rand::random::<f64>() * 0.25;
    (base as f64 * (1.0 + jitter)) as u64
}
```

---

## 🔗 相关文档链接

- [reqwest 官方文档](https://docs.rs/reqwest/)
- [tokio 异步运行时](https://tokio.rs/)
- [async-trait](https://docs.rs/async-trait/)
- [anyhow 错误处理](https://docs.rs/anyhow/)
- [HTTP 状态码参考](https://http.cat/)
- [JSON Schema 规范](https://json-schema.org/)

---

## 📝 检查清单

在实现网络功能时检查以下项目：

- [ ] 使用了 `reqwest::Client` 吗？
- [ ] 异步函数使用了 `async` 关键字吗？
- [ ] 网络操作使用了 `await` 吗？
- [ ] 是否处理了错误（`?` 或 `match`）？
- [ ] 是否实现了重试逻辑？
- [ ] 是否检查了 HTTP 状态码？
- [ ] 是否使用了 `tokio::time::sleep` 而不是 `std::thread::sleep`？
- [ ] 是否验证了输入参数？
- [ ] 是否有日志记录？
- [ ] 是否测试了错误情况？

