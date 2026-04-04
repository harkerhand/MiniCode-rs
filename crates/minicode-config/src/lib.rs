use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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

#[derive(Debug, Clone)]
pub struct ActiveSessionContext {
    pub cwd: PathBuf,
    pub session_id: String,
}

fn active_session_cell() -> &'static Mutex<Option<ActiveSessionContext>> {
    static CELL: OnceLock<Mutex<Option<ActiveSessionContext>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// 设置当前进程内的活动会话上下文。
pub fn set_active_session_context(cwd: PathBuf, session_id: String) {
    if let Ok(mut slot) = active_session_cell().lock() {
        *slot = Some(ActiveSessionContext { cwd, session_id });
    }
}

/// 获取当前进程内的活动会话上下文。
pub fn get_active_session_context() -> Option<ActiveSessionContext> {
    active_session_cell()
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}

/// 返回 mini-code 配置目录 `~/.mini-code`。
pub fn mini_code_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mini-code")
}

/// 返回全局设置文件路径。
pub fn mini_code_settings_path() -> PathBuf {
    mini_code_dir().join("settings.json")
}

/// 返回历史记录文件路径。
pub fn mini_code_history_path() -> PathBuf {
    mini_code_dir().join("history.json")
}

/// 返回权限存储文件路径。
pub fn mini_code_permissions_path() -> PathBuf {
    mini_code_dir().join("permissions.json")
}

/// 返回全局 MCP 配置文件路径。
pub fn mini_code_mcp_path() -> PathBuf {
    mini_code_dir().join("mcp.json")
}

/// 返回兼容 Claude 配置路径。
pub fn claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude/settings.json")
}

/// 返回项目级 MCP 配置路径。
pub fn project_mcp_path(cwd: &Path) -> PathBuf {
    cwd.join(".mcp.json")
}

/// 返回项目级会话目录: .mini-code/sessions/
pub fn project_sessions_dir(cwd: &Path) -> PathBuf {
    cwd.join(".mini-code/sessions")
}

/// 返回特定会话目录: .mini-code/sessions/{session_id}/
pub fn project_session_dir(cwd: &Path, session_id: &str) -> PathBuf {
    project_sessions_dir(cwd).join(session_id)
}

/// 返回会话索引路径: .mini-code/sessions/index.json
pub fn project_sessions_index(cwd: &Path) -> PathBuf {
    project_sessions_dir(cwd).join("index.json")
}

/// 返回会话元数据路径: .mini-code/sessions/{session_id}/metadata.json
pub fn project_session_metadata_path(cwd: &Path, session_id: &str) -> PathBuf {
    project_session_dir(cwd, session_id).join("metadata.json")
}

/// 返回会话对话历史路径: .mini-code/sessions/{session_id}/conversation.json
pub fn project_session_conversation_path(cwd: &Path, session_id: &str) -> PathBuf {
    project_session_dir(cwd, session_id).join("conversation.json")
}

/// 返回会话权限文件路径: .mini-code/sessions/{session_id}/permissions.json
pub fn project_session_permissions_path(cwd: &Path, session_id: &str) -> PathBuf {
    project_session_dir(cwd, session_id).join("permissions.json")
}

/// 返回当前会话文件路径: .mini-code/current_session.json
pub fn project_current_session_path(cwd: &Path) -> PathBuf {
    cwd.join(".mini-code/current_session.json")
}

/// 读取 JSON 文件，不存在时返回默认值。
fn read_json_file<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}

/// 将 overlay 设置覆盖并合并到 base 设置中。
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

/// 读取指定路径中的 MCP 服务器配置表。
fn read_mcp_servers(path: &Path) -> Result<HashMap<String, McpServerConfig>> {
    Ok(read_json_file::<McpConfigFile>(path)?.mcp_servers)
}

/// 按优先级加载并合并最终生效设置。
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

/// 将更新内容合并后写回全局设置文件。
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

/// 从配置与环境变量构建运行时配置。
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

/// 读取指定作用域（user/project）的 MCP 服务器配置。
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

/// 保存指定作用域（user/project）的 MCP 服务器配置。
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
