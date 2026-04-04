# Minicode-rs Architecture

This document explains the role of each crate in the workspace and how they fit together.

## High-Level Layers

1. App Entry
- `apps/minicode`: CLI entrypoint, command routing, runtime wiring, and TUI startup.

2. Interaction Layer
- `minicode-tui`: terminal UI, input handling, transcript rendering, approval dialogs, and turn orchestration.
- `minicode-cli-commands`: built-in slash commands (`/help`, `/status`, `/model`, etc.).
- `minicode-shortcuts`: maps shortcut-style slash commands (`/ls`, `/grep`, `/read`, etc.) to tool calls.

3. Agent Layer
- `minicode-agent-core`: agent loop and model adapters (tool-call loop, progress/final message handling).
- `minicode-mock-model`: deterministic mock `ModelAdapter` for testing and local development.

4. Tooling Layer
- `minicode-tool`: core tool abstractions (`Tool`, `ToolRegistry`, validation, execution, disposal).
- `minicode-tools-runtime`: built-in runtime tools (`read_file`, `edit_file`, `run_command`, etc.) and registry assembly.
- `minicode-mcp`: MCP client/bootstrap and dynamic MCP-backed tool generation.
- `minicode-workspace`: path resolution and workspace-bound path safety checks.
- `minicode-file-review`: diff generation + reviewed file write flow.
- `minicode-background-tasks`: in-memory tracking for background shell tasks.

5. Policy & State Layer
- `minicode-permissions`: path/command/edit approval policy, prompting, persistence.
- `minicode-config`: config loading/merging, runtime config construction, MCP config I/O.
- `minicode-history`: command/input history persistence.
- `minicode-skills`: skill discovery, load/install/remove management.
- `minicode-prompt`: system prompt construction, skill/MCP summaries.

6. Shared Contracts Layer
- `minicode-types`: shared agent/model protocol types (`ChatMessage`, `AgentStep`, `ModelAdapter`).
- `minicode-core`: re-export facade for common foundational crates (config/history/prompt/types).

## Crate-by-Crate Responsibilities

### `apps/minicode`
- Owns process entrypoint (`main`/`real_main`).
- Parses CLI subcommands (`install`, `mcp`, `skills`, `help`).
- Builds runtime dependencies: tool registry, permissions, model adapter, TUI args.

### `minicode-core`
- Thin facade crate.
- Re-exports:
  - `minicode-config`
  - `minicode-history`
  - `minicode-prompt`
  - `minicode-types`
- Keeps imports in upper layers simpler and more stable.

### `minicode-types`
- Defines core message protocol between UI, agent loop, and model adapter.
- Declares `ModelAdapter` trait for pluggable model backends.

### `minicode-config`
- Defines settings/MCP config structs.
- Loads and merges settings from user/project/compat scopes.
- Builds validated runtime config (model, auth, base URL, MCP servers).

### `minicode-history`
- Loads/saves recent interaction history.
- Applies bounded retention for history size.

### `minicode-prompt`
- Builds the final system prompt text.
- Injects cwd, permission summary, skills, MCP server summaries, optional CLAUDE.md content.

### `minicode-skills`
- Discovers skills from project/user/compat directories.
- Loads skill content and metadata.
- Installs/removes managed skills by scope.

### `minicode-install`
- Interactive installer/wizard.
- Writes initial settings and launcher script (`~/.local/bin/minicode`).

### `minicode-manage`
- Handles management command family:
  - `minicode mcp ...`
  - `minicode skills ...`
- Performs add/list/remove operations for MCP servers and skills.

### `minicode-cli-commands`
- Handles local slash commands that do not require model reasoning.
- Provides help text, command matching, and status/config introspection commands.

### `minicode-shortcuts`
- Parses shortcut-form commands and transforms them to tool invocation payloads.
- Keeps input-to-tool mapping logic separate from UI and agent loop.

### `minicode-permissions`
- Central approval engine for:
  - path access outside workspace
  - command execution
  - file edit application
- Supports allow/deny once, per-turn, persistent patterns, and feedback-driven denial.

### `minicode-tool`
- Defines the runtime tool contract and result model.
- Maintains dynamic tool registry, input schema compilation/validation, and execution dispatch.

### `minicode-workspace`
- Resolves tool paths relative to cwd.
- Enforces workspace boundary when permission manager is absent.
- Delegates access checks to permission manager when present.

### `minicode-file-review`
- Builds unified diffs for pending file edits.
- Applies edits through permission-reviewed flow.

### `minicode-background-tasks`
- Registers and tracks background shell tasks (`task_id`, pid, status, cwd).
- Refreshes status from process liveness.

### `minicode-mcp`
- Starts MCP stdio clients and negotiates protocol.
- Lists MCP tools/resources/prompts and exposes them as dynamic tools.
- Adds utility tools (`list_mcp_resources`, `read_mcp_resource`, `list_mcp_prompts`, `get_mcp_prompt`).

### `minicode-tools-runtime`
- Implements built-in tools:
  - user interaction (`ask_user`)
  - filesystem (`list_files`, `read_file`, `write_file`, `modify_file`, `edit_file`, `patch_file`)
  - search (`grep_files`)
  - shell execution (`run_command`)
  - skill loading (`load_skill`)
- Assembles full `ToolRegistry`, optionally extending it with MCP-backed tools.

### `minicode-agent-core`
- Runs the turn loop over model outputs and tool calls.
- Handles progress/final message semantics, retries, fallback behavior.
- Contains Anthropic adapter implementation (`AnthropicModelAdapter`).

### `minicode-mock-model`
- Provides a mock `ModelAdapter` implementation for predictable local testing.
- Produces synthetic tool calls/responses based on slash-style input.

### `minicode-tui`
- Owns terminal UX:
  - input editing/history
  - transcript rendering
  - approval dialog interaction
  - submit/turn lifecycle
- Bridges user actions to local commands, shortcuts, tools, and agent turn execution.

## Runtime Data Flow (Typical Turn)

1. User enters input in `minicode-tui`.
2. TUI tries local slash handling (`minicode-cli-commands`) and shortcut mapping (`minicode-shortcuts`).
3. If model turn is needed, TUI calls `minicode-agent-core::run_agent_turn`.
4. Agent invokes tools through `minicode-tool::ToolRegistry`.
5. Tool runtime (`minicode-tools-runtime`) may:
- read/write files (`minicode-workspace`, `minicode-file-review`)
- run commands (guarded by `minicode-permissions`)
- call MCP tools (`minicode-mcp`)
6. Events stream back to TUI transcript until final assistant output.

