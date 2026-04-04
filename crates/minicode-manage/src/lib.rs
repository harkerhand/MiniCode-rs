use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use minicode_core::config::{
    McpServerConfig, load_scoped_mcp_servers, mini_code_mcp_path, project_mcp_path,
    save_scoped_mcp_servers,
};
use minicode_skills::{discover_skills, install_skill, remove_managed_skill};

/// 列出 MCP 服务
pub async fn list_mcp_servers(cwd: &Path, scope: &str) -> Result<bool> {
    let servers = load_scoped_mcp_servers(scope, cwd)?;

    if servers.is_empty() {
        let path = if scope == "project" {
            project_mcp_path(cwd)
        } else {
            mini_code_mcp_path()
        };
        println!("No MCP servers configured in {}.", path.display());
        return Ok(true);
    }

    for (name, server) in servers {
        let args = server.args.unwrap_or_default().join(" ");
        let protocol = server
            .protocol
            .as_deref()
            .map(|p| format!(" protocol={}", p))
            .unwrap_or_default();

        println!("{}: {} {}{}", name, server.command, args, protocol);
    }
    Ok(true)
}

/// 添加 MCP 服务
pub async fn add_mcp_server(
    cwd: &Path,
    scope: &str,
    name: String,
    protocol: Option<String>,
    env_vars: HashMap<String, serde_json::Value>,
    command: Vec<String>,
) -> Result<bool> {
    if command.is_empty() {
        return Err(anyhow!("Missing MCP command"));
    }

    let mut existing = load_scoped_mcp_servers(scope, cwd)?;
    existing.insert(
        name.clone(),
        McpServerConfig {
            command: command[0].clone(),
            args: if command.len() > 1 {
                Some(command[1..].to_vec())
            } else {
                Some(vec![])
            },
            env: if env_vars.is_empty() {
                None
            } else {
                Some(env_vars)
            },
            cwd: None,
            enabled: None,
            protocol,
        },
    );

    save_scoped_mcp_servers(scope, cwd, existing)?;
    println!("Added MCP server {}", name);
    Ok(true)
}

/// 移除 MCP 服务
pub async fn remove_mcp_server(cwd: &Path, scope: &str, name: String) -> Result<bool> {
    let mut existing = load_scoped_mcp_servers(scope, cwd)?;

    if existing.remove(&name).is_none() {
        println!("MCP server {} not found", name);
        return Ok(true);
    }

    save_scoped_mcp_servers(scope, cwd, existing)?;
    println!("Removed MCP server {}", name);
    Ok(true)
}

/// 列出技能
pub async fn list_skills(cwd: &Path) -> Result<bool> {
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
    cwd: &Path,
    scope: &str,
    path: String,
    name: Option<String>,
) -> Result<bool> {
    let (installed_name, target) = install_skill(cwd, &path, name, scope)?;
    println!("Installed skill {} at {}", installed_name, target);
    Ok(true)
}

/// 移除技能
pub async fn remove_skill(cwd: &Path, scope: &str, name: String) -> Result<bool> {
    let (removed, target) = remove_managed_skill(cwd, &name, scope)?;

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
