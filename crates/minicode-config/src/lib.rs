use std::collections::HashMap;
use std::fs;
use std::path::Path;
mod runtime;
use anyhow::{Result, anyhow};
pub use runtime::*;
use serde::{Deserialize, Serialize};
mod path;
pub use path::*;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    #[serde(default)]
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, serde_json::Value>>,
    pub url: Option<String>,
    pub headers: Option<HashMap<String, serde_json::Value>>,
    pub cwd: Option<String>,
    pub enabled: Option<bool>,
    pub protocol: Option<String>,
}

impl McpServerConfig {
    pub fn new(
        protocol: Option<String>,
        env_vars: HashMap<String, serde_json::Value>,
        url: Option<String>,
        headers: HashMap<String, serde_json::Value>,
        command: Vec<String>,
    ) -> Result<Self> {
        if url.is_some() && !command.is_empty() {
            return Err(anyhow!(
                "Cannot set both remote URL and local command. Choose one."
            ));
        }
        if url.is_none() && command.is_empty() {
            return Err(anyhow!("Missing MCP command or --url"));
        }
        Ok(Self {
            command: if command.is_empty() {
                String::new()
            } else {
                command[0].clone()
            },
            args: if command.len() > 1 {
                Some(command[1..].to_vec())
            } else if command.is_empty() {
                None
            } else {
                Some(vec![])
            },
            env: if env_vars.is_empty() {
                None
            } else {
                Some(env_vars)
            },
            url,
            headers: if headers.is_empty() {
                None
            } else {
                Some(headers)
            },
            cwd: None,
            enabled: None,
            protocol,
        })
    }
}

/// 获取当前进程内缓存的运行时配置（若已初始化）。
pub fn runtime_config() -> RuntimeConfig {
    runtime_store()
        .runtime_config
        .read()
        .map(|c| c.clone())
        .unwrap_or_default()
}
/// 将运行时配置写入进程内缓存。
pub fn modify_runtime_config(config: RuntimeConfig) {
    if let Ok(mut guard) = runtime_store().runtime_config.write() {
        *guard = config;
    }
}

/// 读取 JSON 文件，不存在时返回默认值。
fn read_json_file<T: for<'de> Deserialize<'de> + Default>(path: impl AsRef<Path>) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerConfig>,
}

/// 读取指定路径中的 MCP 服务器配置表。
fn read_mcp_servers(path: impl AsRef<Path>) -> Result<HashMap<String, McpServerConfig>> {
    Ok(read_json_file::<McpConfigFile>(path)?.mcp_servers)
}

/// 按优先级加载并合并最终生效设置。
pub fn config_from_file(cwd: impl AsRef<Path>) -> Result<RuntimeConfig> {
    let global_mcp = read_mcp_servers(mini_code_mcp_path())?;
    let project_mcp = read_mcp_servers(project_mcp_path(cwd))?;
    let mut config = read_json_file::<RuntimeConfig>(&mini_code_settings_path())?;
    config.mcp_servers = global_mcp.into_iter().chain(project_mcp).collect();
    Ok(config)
}

/// 将更新内容合并后写回全局设置文件。
pub fn save_minicode_settings(config: &RuntimeConfig) -> Result<()> {
    let path = mini_code_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(config)?))?;
    Ok(())
}

/// 从配置与环境变量构建运行时配置（不读写单例缓存）。
pub fn build_runtime_config(cwd: impl AsRef<Path>) -> Result<RuntimeConfig> {
    let mut config = config_from_file(cwd)?;
    let env = std::env::vars().collect::<HashMap<_, _>>();

    if let Some(model) = std::env::var("MINI_CODE_MODEL")
        .ok()
        .or_else(|| env.get("ANTHROPIC_MODEL").cloned())
    {
        config.model = model;
    }
    if let Some(base_url) = std::env::var("MINI_CODE_BASE_URL")
        .ok()
        .or_else(|| env.get("ANTHROPIC_BASE_URL").cloned())
    {
        config.base_url = base_url;
    }
    if let Some(max_token_window) = std::env::var("MINI_CODE_MAX_TOKEN_WINDOW")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
    {
        config.max_token_window = Some(max_token_window);
    }
    if let Some(auth_token) = std::env::var("ANTHROPIC_AUTH_TOKEN")
        .ok()
        .or_else(|| env.get("ANTHROPIC_AUTH_TOKEN").cloned())
    {
        config.auth_token = Some(auth_token);
    }
    if let Some(api_key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .or_else(|| env.get("ANTHROPIC_API_KEY").cloned())
    {
        config.api_key = Some(api_key);
    }

    if config.model.trim().is_empty() {
        return Err(anyhow!(
            "No model configured. Set ~/.mini-code/settings.json or ANTHROPIC_MODEL."
        ));
    }
    if config.auth_token.is_none() && config.api_key.is_none() {
        return Err(anyhow!(
            "No auth configured. Set ANTHROPIC_AUTH_TOKEN or ANTHROPIC_API_KEY."
        ));
    }

    Ok(config)
}

/// 读取指定作用域（user/project）的 MCP 服务器配置。
pub fn load_scoped_mcp_servers(
    project: bool,
    cwd: impl AsRef<Path>,
) -> Result<HashMap<String, McpServerConfig>> {
    let path = if project {
        project_mcp_path(cwd)
    } else {
        mini_code_mcp_path()
    };
    read_mcp_servers(&path)
}

/// 读取 MCP token 文件，返回 name -> token 的映射。
pub fn read_mcp_tokens() -> Result<HashMap<String, String>> {
    let path = mini_code_mcp_tokens_path();
    read_json_file::<HashMap<String, String>>(path)
}

/// 保存 MCP token 到文件。
pub fn save_mcp_tokens(tokens: &HashMap<String, String>) -> Result<()> {
    let path = mini_code_mcp_tokens_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(tokens)?))?;
    Ok(())
}

/// 获取指定 MCP 服务器的已存储 token（若有）。
pub fn get_mcp_token(name: &str) -> Option<String> {
    read_mcp_tokens().ok().and_then(|t| t.get(name).cloned())
}

/// 存储指定 MCP 服务器的 token。
pub fn set_mcp_token(name: &str, token: &str) -> Result<()> {
    let mut tokens = read_mcp_tokens().unwrap_or_default();
    tokens.insert(name.to_string(), token.to_string());
    save_mcp_tokens(&tokens)
}

/// 删除指定 MCP 服务器的已存储 token。
pub fn remove_mcp_token(name: &str) -> Result<()> {
    let mut tokens = read_mcp_tokens().unwrap_or_default();
    tokens.remove(name);
    save_mcp_tokens(&tokens)
}

/// 保存指定作用域（user/project）的 MCP 服务器配置。
pub fn save_scoped_mcp_servers(
    project: bool,
    cwd: impl AsRef<Path>,
    servers: HashMap<String, McpServerConfig>,
) -> Result<()> {
    let path = if project {
        project_mcp_path(cwd)
    } else {
        mini_code_mcp_path()
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = serde_json::json!({ "mcpServers": servers });
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&payload)?),
    )?;
    Ok(())
}
