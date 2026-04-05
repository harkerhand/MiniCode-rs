use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use minicode_config::{
    McpServerConfig, load_scoped_mcp_servers, mini_code_mcp_path, project_mcp_path,
    save_scoped_mcp_servers,
};
use minicode_skills::{discover_skills, install_skill, remove_managed_skill};

/// 列出 MCP 服务
pub async fn list_mcp_servers(cwd: impl AsRef<Path>, project: bool) -> Result<bool> {
    let servers = load_scoped_mcp_servers(project, cwd.as_ref())?;

    if servers.is_empty() {
        let path = if project {
            project_mcp_path(cwd.as_ref())
        } else {
            mini_code_mcp_path()
        };
        println!("No MCP servers configured in {}.", path.display());
        return Ok(true);
    }

    for (name, server) in servers {
        let endpoint = if let Some(url) = server.url.as_deref() {
            url.to_string()
        } else {
            let args = server.args.unwrap_or_default().join(" ");
            if args.is_empty() {
                server.command
            } else {
                format!("{} {}", server.command, args)
            }
        };
        let protocol = server
            .protocol
            .as_deref()
            .map(|p| format!(" protocol={}", p))
            .unwrap_or_default();

        println!("{}: {}{}", name, endpoint, protocol);
    }
    Ok(true)
}

/// 添加 MCP 服务
pub async fn add_mcp_server(
    cwd: impl AsRef<Path>,
    project: bool,
    name: String,
    mcp_config: McpServerConfig,
) -> Result<bool> {
    let mut existing = load_scoped_mcp_servers(project, cwd.as_ref())?;
    existing.insert(name.clone(), mcp_config);

    save_scoped_mcp_servers(project, cwd.as_ref(), existing)?;
    println!("Added MCP server {}", name);
    Ok(true)
}

/// 移除 MCP 服务
pub async fn remove_mcp_server(cwd: impl AsRef<Path>, project: bool, name: String) -> Result<bool> {
    let mut existing = load_scoped_mcp_servers(project, cwd.as_ref())?;

    if existing.remove(&name).is_none() {
        println!("MCP server {} not found", name);
        return Ok(true);
    }

    save_scoped_mcp_servers(project, cwd.as_ref(), existing)?;
    println!("Removed MCP server {}", name);
    Ok(true)
}

/// 列出技能
pub async fn list_skills(cwd: impl AsRef<Path>) -> Result<bool> {
    let skills = discover_skills(cwd);
    if skills.is_empty() {
        println!("No skills discovered.");
        return Ok(true);
    }

    for skill in skills {
        println!("{}: {} ({})", skill.name, skill.description, skill.path);
    }
    Ok(true)
}

/// 安装技能
pub async fn add_skill(
    cwd: impl AsRef<Path>,
    project: bool,
    path: String,
    name: Option<String>,
) -> Result<bool> {
    let (installed_name, target) = install_skill(cwd, &path, name, project)?;
    println!("Installed skill {} at {}", installed_name, target);
    Ok(true)
}

/// 移除技能
pub async fn remove_skill(cwd: impl AsRef<Path>, project: bool, name: String) -> Result<bool> {
    let (removed, target) = remove_managed_skill(cwd, &name, project)?;

    if !removed {
        println!("Skill {} not found at {}", name, target);
        return Ok(true);
    }

    println!("Removed skill {} from {}", name, target);
    Ok(true)
}

/// 解析 KEY=VALUE 形式的环境变量
pub fn parse_env_pairs(entries: &[String]) -> Result<HashMap<String, serde_json::Value>> {
    let mut env = HashMap::new();

    for entry in entries {
        let Some(eq_idx) = entry.find('=') else {
            return Err(anyhow!(
                "Invalid environment variable format: {} (expected KEY=VALUE)",
                entry
            ));
        };

        let key = entry[..eq_idx].trim();
        let value = entry[eq_idx + 1..].to_string();

        if key.is_empty() {
            return Err(anyhow!(
                "Invalid environment variable: empty key in {}",
                entry
            ));
        }

        env.insert(key.to_string(), serde_json::Value::String(value));
    }

    Ok(env)
}
