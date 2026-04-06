use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};

use crate::{McpServerConfig, build_runtime_config};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    pub model: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(rename = "maxOutputTokens")]
    pub max_token_window: Option<u32>,
    #[serde(default)]
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
}

pub struct RuntimeStore {
    pub cwd: PathBuf,
    pub session_id: String,
    pub session_started_at: DateTime<Utc>,
    pub runtime_config: Arc<RwLock<RuntimeConfig>>,
}

static RUNTIME_STORE: OnceLock<RuntimeStore> = OnceLock::new();

pub fn init_runtime_store(cwd: impl AsRef<Path>, session_id: impl AsRef<str>) {
    let runtime_config = build_runtime_config(cwd.as_ref()).unwrap_or_default();
    let store = RuntimeStore {
        cwd: cwd.as_ref().to_path_buf(),
        session_id: session_id.as_ref().to_string(),
        session_started_at: Utc::now(),
        runtime_config: Arc::new(RwLock::new(runtime_config)),
    };
    let _ = RUNTIME_STORE.set(store);
}

pub fn runtime_store() -> &'static RuntimeStore {
    RUNTIME_STORE.get().expect("Runtime store not initialized")
}
