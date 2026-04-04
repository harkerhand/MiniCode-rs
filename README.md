# MiniCode (Rust)

[原始仓库（TypeScript 版本）](https://github.com/LiuMengxuan04/MiniCode)

本文档仅保留 Rust 版本的运行相关说明。

## Build

```bash
cd MiniCode-rs
cargo build -p minicode
```

## Install

```bash
cargo run -p minicode -- install
```

安装向导会写入配置并生成启动脚本 `~/.local/bin/minicode`。

## Run

开发模式：

```bash
cargo run -p minicode
```

mock 模型：

```bash
MINI_CODE_MODEL_MODE=mock cargo run -p minicode
```

安装后：

```bash
minicode
```

## Runtime Config

默认配置读取路径：

- `~/.mini-code/settings.json`
- `~/.mini-code/mcp.json`
- 项目级 `.mcp.json`
- `./.claude/settings.json`（兼容读取）

配置示例：

```json
{
  "model": "your-model-name",
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
    "ANTHROPIC_AUTH_TOKEN": "your-token",
    "ANTHROPIC_MODEL": "your-model-name"
  },
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
    }
  }
}
```

## CLI

- `minicode mcp list [--project]`
- `minicode mcp add <name> [--project] [--protocol <auto|content-length|newline-json>] [--env KEY=VALUE ...] -- <command> [args...]`
- `minicode mcp remove <name> [--project]`
- `minicode skills list`
- `minicode skills add <path-to-skill-or-dir> [--name <name>] [--project]`
- `minicode skills remove <name> [--project]`

## Dev Checks

```bash
cargo fmt
cargo test
```
