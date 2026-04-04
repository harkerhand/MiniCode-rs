use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, serde_json::Value>>,
    pub cwd: Option<String>,
    pub enabled: Option<bool>,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MiniCodeSettings {
    pub env: Option<HashMap<String, serde_json::Value>>,
    pub model: Option<String>,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u32>,
    #[serde(rename = "mcpServers")]
    pub mcp_servers: Option<HashMap<String, McpServerConfig>>,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub model: String,
    pub base_url: String,
    pub auth_token: Option<String>,
    pub api_key: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub mcp_servers: HashMap<String, McpServerConfig>,
    pub source_summary: String,
}

pub fn mini_code_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mini-code")
}

pub fn mini_code_settings_path() -> PathBuf {
    mini_code_dir().join("settings.json")
}

pub fn mini_code_history_path() -> PathBuf {
    mini_code_dir().join("history.json")
}

pub fn mini_code_permissions_path() -> PathBuf {
    mini_code_dir().join("permissions.json")
}

pub fn mini_code_mcp_path() -> PathBuf {
    mini_code_dir().join("mcp.json")
}

pub fn claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude/settings.json")
}

pub fn project_mcp_path(cwd: &Path) -> PathBuf {
    cwd.join(".mcp.json")
}

fn read_json_file<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}

fn merge_settings(base: MiniCodeSettings, overlay: MiniCodeSettings) -> MiniCodeSettings {
    let mut env = base.env.unwrap_or_default();
    env.extend(overlay.env.unwrap_or_default());

    let mut mcp = base.mcp_servers.unwrap_or_default();
    for (k, v) in overlay.mcp_servers.unwrap_or_default() {
        mcp.insert(k, v);
    }

    MiniCodeSettings {
        env: if env.is_empty() { None } else { Some(env) },
        model: overlay.model.or(base.model),
        max_output_tokens: overlay.max_output_tokens.or(base.max_output_tokens),
        mcp_servers: if mcp.is_empty() { None } else { Some(mcp) },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerConfig>,
}

fn read_mcp_servers(path: &Path) -> Result<HashMap<String, McpServerConfig>> {
    Ok(read_json_file::<McpConfigFile>(path)?.mcp_servers)
}

pub fn load_effective_settings(cwd: &Path) -> Result<MiniCodeSettings> {
    let claude = read_json_file::<MiniCodeSettings>(&claude_settings_path())?;
    let global_mcp = MiniCodeSettings {
        mcp_servers: Some(read_mcp_servers(&mini_code_mcp_path())?),
        ..MiniCodeSettings::default()
    };
    let project_mcp = MiniCodeSettings {
        mcp_servers: Some(read_mcp_servers(&project_mcp_path(cwd))?),
        ..MiniCodeSettings::default()
    };
    let mini = read_json_file::<MiniCodeSettings>(&mini_code_settings_path())?;

    Ok(merge_settings(
        merge_settings(merge_settings(claude, global_mcp), project_mcp),
        mini,
    ))
}

pub fn save_minicode_settings(updates: MiniCodeSettings) -> Result<()> {
    let path = mini_code_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = read_json_file::<MiniCodeSettings>(&path)?;
    let merged = merge_settings(existing, updates);
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&merged)?),
    )?;
    Ok(())
}

pub fn load_runtime_config(cwd: &Path) -> Result<RuntimeConfig> {
    let effective = load_effective_settings(cwd)?;
    let mut env = std::env::vars().collect::<HashMap<_, _>>();
    if let Some(extra) = &effective.env {
        for (k, v) in extra {
            env.insert(k.clone(), v.to_string().trim_matches('"').to_string());
        }
    }

    let model = std::env::var("MINI_CODE_MODEL")
        .ok()
        .or(effective.model)
        .or_else(|| env.get("ANTHROPIC_MODEL").cloned())
        .unwrap_or_default();

    let base_url = env
        .get("ANTHROPIC_BASE_URL")
        .cloned()
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());

    let auth_token = env
        .get("ANTHROPIC_AUTH_TOKEN")
        .cloned()
        .filter(|x| !x.is_empty());
    let api_key = env
        .get("ANTHROPIC_API_KEY")
        .cloned()
        .filter(|x| !x.is_empty());

    if model.trim().is_empty() {
        return Err(anyhow!(
            "No model configured. Set ~/.mini-code/settings.json or ANTHROPIC_MODEL."
        ));
    }
    if auth_token.is_none() && api_key.is_none() {
        return Err(anyhow!(
            "No auth configured. Set ANTHROPIC_AUTH_TOKEN or ANTHROPIC_API_KEY."
        ));
    }

    Ok(RuntimeConfig {
        model,
        base_url,
        auth_token,
        api_key,
        max_output_tokens: effective.max_output_tokens,
        mcp_servers: effective.mcp_servers.unwrap_or_default(),
        source_summary: format!(
            "config: {} > {} > process.env",
            mini_code_settings_path().display(),
            claude_settings_path().display()
        ),
    })
}

pub fn load_scoped_mcp_servers(
    scope: &str,
    cwd: &Path,
) -> Result<HashMap<String, McpServerConfig>> {
    let path = if scope == "project" {
        project_mcp_path(cwd)
    } else {
        mini_code_mcp_path()
    };
    read_mcp_servers(&path)
}

pub fn save_scoped_mcp_servers(
    scope: &str,
    cwd: &Path,
    servers: HashMap<String, McpServerConfig>,
) -> Result<()> {
    let path = if scope == "project" {
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
