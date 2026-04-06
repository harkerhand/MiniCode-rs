use std::sync::Arc;

use anyhow::Result;
use minicode_config::runtime_config;
use minicode_mcp::{create_mcp_backed_tools, set_mcp_logging_enabled};
use minicode_skills::discover_skills;
use minicode_tool::{Tool, ToolContext, ToolRegistry};
mod act;
mod command;
mod file;
mod web;
use act::*;
use command::*;
use file::*;
use web::*;

/// 控制 MCP 启动阶段日志开关。
pub fn set_mcp_startup_logging_enabled(enabled: bool) {
    set_mcp_logging_enabled(enabled);
}

/// 创建默认工具注册表，并按配置注入 MCP 工具。
pub async fn create_default_tool_registry() -> Result<ToolRegistry> {
    let skills = discover_skills();
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(AskUserTool),
        Arc::new(ListFilesTool),
        Arc::new(GrepFilesTool),
        Arc::new(ReadFileTool),
        Arc::new(WriteLikeTool {
            name: "write_file",
            description: "写入 UTF-8 文本文件。",
        }),
        Arc::new(WriteLikeTool {
            name: "modify_file",
            description: "替换文件全部内容（带 diff 审核）。",
        }),
        Arc::new(EditFileTool),
        Arc::new(PatchFileTool),
        Arc::new(RunCommandTool),
        Arc::new(WebSearchTool),
        Arc::new(WebFetchTool),
        Arc::new(LoadSkillTool),
    ];
    let runtime = runtime_config();
    let mcp = create_mcp_backed_tools(&runtime.mcp_servers).await;
    tools.extend(mcp.tools);
    let mcp_server_summaries = mcp.servers;
    let mcp_disposer = mcp.disposer;

    Ok(ToolRegistry::new(
        tools,
        skills,
        mcp_server_summaries,
        mcp_disposer,
    ))
}
