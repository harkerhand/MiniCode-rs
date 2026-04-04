# Minicode-rs 项目依赖和文本处理功能分析报告

## 📋 项目概览
- **项目名称**: Minicode-rs
- **类型**: Rust Monorepo (Cargo Workspace)
- **主要依赖管理**: 工作区级 Cargo.toml
- **成员数量**: 20+ crates

## 🔍 1. HTML 解析库集成情况

### ❌ **已集成的 HTML 解析库**
**当前状态**: 项目中**未集成**任何专门的 HTML 解析库

**验证结果**:
- 无 `scraper` / `select` / `html5ever` / `markup5ever`
- 无 `ammonia` (HTML 清理库)
- 无 `htmlescape` / `html_escape`
- 无 `html2text` (HTML 转纯文本)

**Cargo.lock 验证**:
```
√ 已检查: 依赖树中不存在 HTML 解析相关库
√ 已检查: Cargo.toml 工作区依赖中未定义
```

### 当前的类似功能替代方案:

**1. 基础文本处理 (已有)**
   - `regex` - 正则表达式 (间接依赖，通过 ratatui)
   - `unicode-width` - Unicode 字符宽度计算
   - 标准 Rust `String` 和 `&str` 方法

**2. 网络功能**
   - `reqwest` - HTTP 客户端库 (v0.13.2)
     - 用于获取网络资源
     - 不包含 HTML 解析功能

## 🛠️ 2. 文本提取和清理实现

### ✅ **已有的文本处理函数**

#### A. 文本提取 (Extract)
**位置**: `crates/minicode-skills/src/lib.rs`
```rust
fn extract_description(markdown: &str) -> String {
    // 功能: 从 Markdown 中提取首段可读描述
    // 功能:
    //   - 规范化 CRLF 到 LF
    //   - 分块处理段落 ("\n\n")
    //   - 过滤空行和标题行
    //   - 去除反引号 (`)
    // 返回: 单行文本摘要
}
```

#### B. 标记清理 (Clean Markup)
**位置**: `crates/minicode-agent-core/src/anthropic_adapter.rs`
```rust
fn parse_assistant_text(content: &str) -> (String, Option<String>) {
    // 功能: 解析助手文本中的 XML/HTML-like 标记
    // 支持标记:
    //   - <final></final> / [FINAL]
    //   - <progress></progress> / [PROGRESS]
    // 返回: (清理后的文本, 标记类型)
}
```

#### C. 字符串分割和解析
**位置**: `crates/minicode-shortcuts/src/lib.rs`
```rust
fn parse_local_tool_shortcut(input: &str) -> Option<LocalToolShortcut> {
    // 功能: 解析斜杠命令语法
    // 分割符: "::" (使用 split_once, splitn)
    // 处理类型:
    //   - /ls, /grep, /read, /write, /edit, /patch, /cmd
    //   - 自动 trim() 和参数验证
}
```

#### D. 名称规范化 (Sanitization)
**位置**: `crates/minicode-mcp/src/lib.rs`
```rust
fn sanitize_segment(value: &str) -> String {
    // 功能: 将字符串转换为安全标识符
    // 操作:
    //   - 转小写
    //   - 保留: 字母/数字, 下划线, 连字符
    //   - 其他字符替换为下划线
}
```

#### E. 文本截断
**位置**: `apps/minicode/src/utils.rs`
```rust
pub fn truncate_log_text(input: &str, max_chars: usize) -> String {
    // 功能: 按字符数截断，追加省略号
    // 处理: Unicode 字符安全 (使用 .chars())
}
```

#### F. 差异生成
**位置**: `crates/minicode-file-review/src/lib.rs`
```rust
pub fn build_unified_diff(file_path: &str, before: &str, after: &str) -> String {
    // 功能: 使用 `similar` 库生成统一 diff 格式
    // 上下文行: 3 行
}
```

## 📦 3. JSON/字符串处理实用工具

### ✅ **已集成的序列化库**

| 库名 | 版本 | 用途 | 状态 |
|-----|------|------|------|
| `serde` | 1.0.228 | JSON 序列化/反序列化 | ✅ 在用 |
| `serde_json` | 1.0.149 | JSON 处理 | ✅ 在用 |
| `jsonschema` | 0.45.0 | JSON Schema 验证 | ✅ 在用 |

### JSON 处理示例

**1. 工具输入验证** (`crates/minicode-tool/src/lib.rs`)
```rust
fn validate_tool_input(validator: &InputValidator, input: &Value) -> Result<(), String> {
    // 使用 jsonschema 按 JSON Schema Draft 7 验证
    // 返回错误信息或验证通过
}
```

**2. 命令参数构建** (`crates/minicode-shortcuts/src/lib.rs`)
```rust
serde_json::json!({ 
    "pattern": parts[0].trim(), 
    "path": parts[1].trim() 
})
// 动态构建 JSON 对象用于工具调用
```

**3. 消息格式转换** (`crates/minicode-agent-core/src/anthropic_adapter.rs`)
```rust
fn parse_anthropic_messages(messages: &[ChatMessage]) 
    -> (String, Vec<AnthropicMessage>) {
    // 将内部消息格式转换为 Anthropic API 格式
    // 包含 JSON 序列化
}
```

## 📚 4. 项目依赖库完整清单

### 工作区级公共依赖 (Cargo.toml)

#### 核心库
- `anyhow` (1.0.102) - 错误处理
- `tokio` (1.51.0) - 异步运行时
- `serde` (1.0.228) + features: derive
- `serde_json` (1.0.149)

#### 网络和序列化
- `reqwest` (0.13.2) - HTTP 客户端
- `httpdate` (1.0.3) - HTTP 日期解析
- `jsonschema` (0.45.0) - JSON Schema 验证

#### 文本和字符串处理
- `shell-words` (1.1.1) - Shell 命令行解析
- `unicode-width` (0.2.2) - Unicode 宽度计算
- `similar` (3.0.0) - 文本差异比较

#### UI 和终端
- `crossterm` (0.29.0) - 终端控制
- `ratatui` (0.30.0) - TUI 框架

#### 工具
- `uuid` (1.23.0) - UUID 生成
- `chrono` (0.4.44) - 日期时间
- `clap` (4.6.0) - CLI 参数解析
- `rand` (0.10.0) - 随机数
- `async-trait` (0.1.89) - 异步 trait
- `futures` (0.3.32) - Future 工具
- `dirs` (6.0.0) - 目录路径

### Crate 特定依赖 (示例)

**minicode-tool**:
```toml
[dependencies]
minicode-types = { workspace = true }
anyhow = { workspace = true }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
jsonschema = { workspace = true }
tokio = { workspace = true, features = ["sync"] }
```

**minicode-file-review**:
```toml
[dependencies]
anyhow = { workspace = true }
minicode-tool = { workspace = true }
similar = { workspace = true }  # ← 用于 diff 生成
```

## 🎯 5. 功能对标与缺口分析

### 现有功能能覆盖的场景

| 需求 | 已有方案 | 库 | 实现位置 |
|-----|--------|-----|--------|
| Markdown 解析 | ❌ | - | - |
| HTML 解析/选择器 | ❌ | - | - |
| HTML → 纯文本转换 | ❌ | - | - |
| XML 处理 | ❌ | - | - |
| 基础字符串清理 | ✅ | 标准库 | minicode-mcp |
| JSON 验证 | ✅ | jsonschema | minicode-tool |
| 差异比较 | ✅ | similar | minicode-file-review |
| 命令行解析 | ✅ | shell-words | minicode-shortcuts |
| HTTP 请求 | ✅ | reqwest | minicode-agent-core |

### 如需添加的功能

1. **HTML 解析** → 需添加 `scraper` 或 `select`
2. **HTML 清理** → 需添加 `ammonia` 或 `html5ever`
3. **Markdown 处理** → 需添加 `markdown` 或 `comrak`
4. **完整的文本规范化** → 需添加 `unicode-normalization`
5. **HTML → Markdown** → 需添加 `html2md` 或自实现

## 📊 6. 依赖统计

```
工作区成员: 20+ crates
工作区级公共依赖: 18 个
Cargo.lock 总体积: ~98.6 KB
锁定版本级别: 精确版本
```

## ✅ 关键发现总结

### 已集成功能
1. ✅ JSON 序列化/反序列化 (完整)
2. ✅ JSON Schema 验证 (Draft 7)
3. ✅ 基础字符串处理 (trim, split, replace)
4. ✅ 文本差异生成 (unified diff)
5. ✅ 正则表达式 (间接依赖)
6. ✅ HTTP 网络请求

### 缺失功能
1. ❌ HTML DOM 解析
2. ❌ CSS 选择器查询
3. ❌ Markdown 解析和处理
4. ❌ 专门的 HTML 清理和转义
5. ❌ XML 处理
6. ❌ HTML → 纯文本/Markdown 转换

## 💡 建议

**如果需要 HTML 处理，推荐方案**:

```toml
# Cargo.toml
[workspace.dependencies]
# 方案 A: 简轻型
scraper = "0.18"  # 带 CSS 选择器的 HTML 解析

# 方案 B: 完整型
ammonia = "4.0"   # HTML 清理和规范化
select = "0.0"    # CSS 选择器查询
select-rs = "0.1" # Alternative

# 方案 C: 多格式支持
html2md = "0.2"   # HTML to Markdown
markdown = "0.3"  # Markdown 解析
```

**如果需要文本提取，当前实现**:
- 已有基础框架在 `minicode-skills` 和 `minicode-agent-core`
- 可复用 `extract_description()` 和 `parse_assistant_text()` 的模式
- 无需额外库就能处理简单情况

