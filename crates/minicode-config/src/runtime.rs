use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock},
    time::SystemTime,
};

use crate::{McpServerConfig, config_from_file};
use minicode_types::ChatMessage;
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
    pub session_started_at: SystemTime,
    pub runtime_config: Arc<RwLock<Option<RuntimeConfig>>>,
    pub runtime_messages: Arc<Mutex<Vec<ChatMessage>>>,
    pub runtime_input_history: Arc<Mutex<Vec<String>>>,
}

static RUNTIME_STORE: OnceLock<RuntimeStore> = OnceLock::new();

pub fn init_runtime_store(cwd: impl AsRef<Path>, session_id: impl AsRef<str>) {
    let runtime_config = config_from_file(&cwd);
    let store = RuntimeStore {
        cwd: cwd.as_ref().to_path_buf(),
        session_id: session_id.as_ref().to_string(),
        session_started_at: SystemTime::now(),
        runtime_config: Arc::new(RwLock::new(runtime_config.ok())),
        runtime_messages: Arc::new(Mutex::new(Vec::new())),
        runtime_input_history: Arc::new(Mutex::new(Vec::new())),
    };
    let _ = RUNTIME_STORE.set(store);
}

pub fn runtime_store() -> &'static RuntimeStore {
    RUNTIME_STORE.get().expect("Runtime store not initialized")
}

pub fn runtime_messages_state() -> Arc<Mutex<Vec<ChatMessage>>> {
    runtime_store().runtime_messages.clone()
}

pub fn runtime_input_history_state() -> Arc<Mutex<Vec<String>>> {
    runtime_store().runtime_input_history.clone()
}
