# MCP 和 Tool 系统文档索引

## 📚 文档导航

本项目提供了完整的 MCP（Model Context Protocol）和工具系统文档。根据你的需求选择对应的文档：

---

## 🚀 快速开始 (推荐从这里开始)

### 对于想快速上手的开发者
👉 **[QUICK_REFERENCE.md](./QUICK_REFERENCE.md)** - 快速参考指南

包含：
- 常用代码模板
- 参数解析速查
- Schema 定义示例
- 错误处理模式
- 内置工具列表
- 常见问题解答

---

## 🔍 深度学习

### 对于想理解设计原理的开发者
👉 **[MCP_TOOL_EXPLORATION.md](./MCP_TOOL_EXPLORATION.md)** - 完整探索指南

包含：
1. **项目结构概览** - Crates 关系和分布
2. **Tool Trait 定义** - 核心接口定义
3. **input_schema 和参数验证** - 验证机制详解
4. **ToolRegistry 工作原理** - 工具注册表细节
5. **内置工具实现示例** - 真实代码示例
6. **MCP 集成工作流** - 完整的 MCP 流程
7. **配置体系** - 配置加载优先级
8. **工具创建工作流** - 创建新工具的步骤
9. **错误处理模式** - 统一的错误处理
10. **超时设置** - 关键超时配置
11. **后台任务支持** - 后台运行机制

---

## 💡 代码示例

### 对于想看具体代码的开发者
👉 **[CODE_EXAMPLES.md](./CODE_EXAMPLES.md)** - 详细代码示例

包含：
- 简单工具 vs 复杂工具对比
- 参数解析模式对比
- Schema 定义最佳实践
- MCP 工具包装流程
- ToolRegistry 执行流程
- 错误处理模式
- JSON-RPC 协议细节
- 协议交换时序图
- 超时处理示例
- MCP 启动流程详解

---

## 🏗️ 架构文档

### 对于想了解整体架构的开发者
👉 **[ARCHITECTURE.md](./ARCHITECTURE.md)** - 架构文档

包含：
- 项目整体架构
- 组件间的关系
- 数据流

---

## 📁 源代码位置

### 直接查看源代码

| 模块 | 路径 | 说明 |
|-----|------|------|
| **Tool 核心** | `crates/minicode-tool/src/lib.rs` | Tool trait 定义、ToolResult、ToolRegistry |
| **内置工具** | `crates/minicode-tools-runtime/src/lib.rs` | 所有默认工具的实现 |
| **MCP 支持** | `crates/minicode-mcp/src/lib.rs` | MCP 客户端、JSON-RPC 通信、工具包装 |
| **配置管理** | `crates/minicode-config/src/lib.rs` | 配置加载、MCP 服务器配置 |
| **系统提示** | `crates/minicode-prompt/src/lib.rs` | 系统提示词生成 |

---

## 🎯 按用户类型推荐路径

### 👤 方案 A: 我是初学者
```
1. 读 QUICK_REFERENCE.md (10 分钟)
   └─ 掌握基本概念和代码模板

2. 浏览 CODE_EXAMPLES.md 中的"简单工具示例" (5 分钟)
   └─ 看真实代码长什么样

3. 查看源代码中的 AskUserTool 和 ListFilesTool (10 分钟)
   └─ 看最简单的两个工具实现

4. 开始写你的第一个工具！
```

---

### 👤 方案 B: 我想完全理解这个系统
```
1. 读 MCP_TOOL_EXPLORATION.md 的第 1-4 章 (20 分钟)
   └─ 理解基本概念

2. 查看 crates/minicode-tool/src/lib.rs (15 分钟)
   └─ 看源代码验证理论

3. 读 MCP_TOOL_EXPLORATION.md 的第 5-7 章 (20 分钟)
   └─ 理解工具和 MCP 集成

4. 阅读 CODE_EXAMPLES.md 的完整内容 (30 分钟)
   └─ 看所有代码示例

5. 查看 crates/minicode-mcp/src/lib.rs (30 分钟)
   └─ 理解 MCP 实现细节

6. 你现在是专家了！
```

---

### 👤 方案 C: 我只想快速找到某个答案
```
使用 QUICK_REFERENCE.md：
- 快速参考表格找到你需要的部分
- 复制对应的代码模板
- 或查看常见问题解答（FAQs）

如果找不到，再查看其他文档。
```

---

## 🔑 关键概念速记

### Tool Trait
```rust
trait Tool {
    fn name() -> &str           // 工具名称
    fn description() -> &str    // 工具描述
    fn input_schema() -> Value  // JSON Schema
    async fn run() -> ToolResult // 执行
}
```

### ToolResult
```rust
struct ToolResult {
    ok: bool,                           // 成功/失败
    output: String,                     // 输出内容
    background_task: Option<...>,       // 后台任务
    await_user: bool,                   // 等待用户
}
```

### ToolRegistry
```rust
impl ToolRegistry {
    async fn execute(tool_name, input, context) // 执行工具
    fn extend_dynamic_tools(tools)               // 动态注册
}
```

---

## 🎓 学习时间估计

| 文档 | 预期时间 | 难度 |
|------|---------|------|
| QUICK_REFERENCE.md | 15-30 分钟 | 低 |
| CODE_EXAMPLES.md | 30-45 分钟 | 中 |
| MCP_TOOL_EXPLORATION.md | 60-90 分钟 | 高 |
| 阅读全部源代码 | 120+ 分钟 | 高 |

---

## ❓ 快速问题查询

### "我想创建新工具"
→ 参考 QUICK_REFERENCE.md 的 "快速创建工具" 部分

### "我想理解 MCP 如何工作"
→ 参考 MCP_TOOL_EXPLORATION.md 的第 6 章 "MCP 集成工作流"

### "我想看 JSON Schema 的定义"
→ 参考 CODE_EXAMPLES.md 的第 3 章 或 QUICK_REFERENCE.md 的 "Schema 定义速查"

### "我遇到了错误，不知道怎么处理"
→ 参考 QUICK_REFERENCE.md 的 "常见错误处理模式"

### "我想了解工具的参数验证"
→ 参考 MCP_TOOL_EXPLORATION.md 的第 3 章或 CODE_EXAMPLES.md 的第 5 章

### "我想知道内置工具有哪些"
→ 参考 QUICK_REFERENCE.md 的 "内置工具列表"

---

## 📞 获取帮助

### 文档找不到答案？
1. 检查本索引文件中的"快速问题查询"
2. 再查看 QUICK_REFERENCE.md 中的常见问题
3. 查看源代码中的现有工具实现
4. 查看项目中的示例用法

---

## 📝 文档维护

这些文档通过以下方式保持最新：
- 源代码变更时同步更新
- 定期审查和改进清晰度
- 根据社区反馈优化

**最后更新**: 2024 年 4 月 5 日

---

## 🚀 立即开始

**推荐第一步**：打开 [QUICK_REFERENCE.md](./QUICK_REFERENCE.md)

祝你使用愉快！
