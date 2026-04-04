# Minicode-rs 架构说明

本文档说明工作区中各个 crate 的职责，以及它们之间的协作关系。

## 架构分层概览

1. 应用入口层
- `apps/minicode`：CLI 入口、命令分发、运行时装配、TUI 启动。

2. 交互层
- `minicode-tui`：终端 UI、输入处理、会话渲染、审批弹窗、回合编排。
- `minicode-cli-commands`：内置斜杠命令（`/help`、`/status`、`/model` 等）。
- `minicode-shortcuts`：把快捷斜杠命令（`/ls`、`/grep`、`/read` 等）映射到工具调用。

3. Agent 层
- `minicode-agent-core`：Agent 回合循环与模型适配（工具调用循环、progress/final 语义）。
- `minicode-mock-model`：用于测试和本地开发的确定性 `ModelAdapter`。

4. 工具层
- `minicode-tool`：工具抽象核心（`Tool`、`ToolRegistry`、入参校验、执行分发）。
- `minicode-tools-runtime`：内置运行时工具实现（`read_file`、`edit_file`、`run_command` 等）与注册表构建。
- `minicode-mcp`：MCP 客户端启动与动态工具注入。
- `minicode-workspace`：路径解析与工作区边界校验。
- `minicode-file-review`：diff 生成与“审批后写入”流程。
- `minicode-background-tasks`：后台 shell 任务状态管理。

5. 策略与状态层
- `minicode-permissions`：路径/命令/编辑审批策略、交互确认与持久化。
- `minicode-config`：配置加载合并、运行时配置构建、MCP 配置读写。
- `minicode-history`：历史输入持久化。
- `minicode-skills`：技能发现、加载、安装、删除。
- `minicode-prompt`：系统提示词拼装，注入技能/MCP/权限信息。

6. 共享契约层
- `minicode-types`：Agent/模型共享协议类型（`ChatMessage`、`AgentStep`、`ModelAdapter`）。
- `minicode-core`：基础能力门面，统一 re-export（config/history/prompt/types）。

## 各 Crate 职责

### `apps/minicode`
- 程序主入口（`main`/`real_main`）。
- 解析 CLI 子命令（`install`、`mcp`、`skills`、`help`）。
- 装配运行时依赖：工具注册表、权限管理器、模型适配器、TUI 参数。

### `minicode-core`
- 轻量门面 crate。
- 对外统一导出：
  - `minicode-config`
  - `minicode-history`
  - `minicode-prompt`
  - `minicode-types`
- 降低上层模块引用复杂度。

### `minicode-types`
- 定义 UI、Agent、模型之间的消息协议类型。
- 定义可插拔模型后端接口 `ModelAdapter`。

### `minicode-config`
- 定义 settings 与 MCP 配置结构。
- 从多作用域加载并合并配置（用户/项目/兼容层）。
- 产出经过校验的运行时配置（模型、鉴权、base URL、MCP 服务）。

### `minicode-history`
- 加载与保存历史输入。
- 对历史条目数量做上限裁剪。

### `minicode-prompt`
- 生成最终系统提示词。
- 注入 cwd、权限摘要、技能列表、MCP 服务摘要、可选 CLAUDE.md 内容。

### `minicode-skills`
- 从项目/用户/兼容目录发现技能。
- 加载技能正文与元数据。
- 按作用域安装/删除托管技能。

### `minicode-install`
- 交互式安装向导。
- 写入初始配置并创建启动脚本（`~/.local/bin/minicode`）。

### `minicode-manage`
- 处理管理命令：
  - `minicode mcp ...`
  - `minicode skills ...`
- 支持 MCP 服务与技能的增删查。

### `minicode-cli-commands`
- 处理无需模型推理的本地斜杠命令。
- 提供帮助、命令匹配、状态与配置路径查询。

### `minicode-shortcuts`
- 解析快捷命令并构造对应工具调用参数。
- 将输入映射逻辑与 UI/Agent 解耦。

### `minicode-permissions`
- 统一审批引擎，覆盖：
  - 工作区外路径访问
  - 命令执行
  - 文件编辑应用
- 支持一次性、回合级、持久化规则与带反馈拒绝。

### `minicode-tool`
- 定义工具运行时契约与结果结构。
- 管理动态工具注册表、schema 编译/校验与执行分发。

### `minicode-workspace`
- 基于 cwd 解析工具目标路径。
- 在无权限管理器时强制工作区边界。
- 在有权限管理器时委托其做访问审批。

### `minicode-file-review`
- 生成统一 diff 预览。
- 按“先审批后写入”流程应用文件改动。

### `minicode-background-tasks`
- 注册并追踪后台 shell 任务（`task_id`、pid、状态、cwd）。
- 基于进程存活情况刷新任务状态。

### `minicode-mcp`
- 启动 MCP stdio 客户端并协商协议。
- 拉取 MCP 工具/资源/Prompt，并注入为动态工具。
- 额外提供工具：
  - `list_mcp_resources`
  - `read_mcp_resource`
  - `list_mcp_prompts`
  - `get_mcp_prompt`

### `minicode-tools-runtime`
- 实现内置工具：
  - 用户交互（`ask_user`）
  - 文件系统（`list_files`、`read_file`、`write_file`、`modify_file`、`edit_file`、`patch_file`）
  - 搜索（`grep_files`）
  - 命令执行（`run_command`）
  - 技能加载（`load_skill`）
- 构建完整 `ToolRegistry`，并在配置存在时接入 MCP 动态工具。

### `minicode-agent-core`
- 执行模型回合循环与工具调用闭环。
- 处理 progress/final 语义、重试与兜底逻辑。
- 包含 Anthropic 模型适配实现（`AnthropicModelAdapter`）。

### `minicode-mock-model`
- 提供 mock `ModelAdapter`，便于可预测测试。
- 根据输入生成模拟工具调用和回复。

### `minicode-tui`
- 负责终端交互体验：
  - 输入编辑与历史
  - 会话渲染
  - 审批弹窗
  - 提交流程与回合生命周期
- 将用户操作桥接到本地命令、快捷映射、工具系统与 agent 回合执行。

## 典型回合数据流

1. 用户在 `minicode-tui` 输入请求。
2. TUI 先尝试本地斜杠处理（`minicode-cli-commands`）和快捷映射（`minicode-shortcuts`）。
3. 若需模型推理，调用 `minicode-agent-core::run_agent_turn`。
4. Agent 通过 `minicode-tool::ToolRegistry` 执行工具。
5. 运行时工具（`minicode-tools-runtime`）可能触发：
- 文件访问（`minicode-workspace` + `minicode-file-review`）
- 命令执行（受 `minicode-permissions` 审批）
- MCP 工具调用（`minicode-mcp`）
6. 事件持续回流到 TUI，直至产出最终助手消息。

