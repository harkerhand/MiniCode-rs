use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use minicode_config::{project_session_conversation_path, runtime_store};
use minicode_types::ChatMessage;
use serde::{Deserialize, Serialize};

/// Generate a unique session ID with timestamp and UUID
pub fn generate_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("sess_{:x}_{}", timestamp, uuid::Uuid::new_v4().simple())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConversationFile {
    messages: Vec<ChatMessage>,
}

fn save_session_messages(
    cwd: impl AsRef<Path>,
    session_id: &str,
    messages: &[ChatMessage],
) -> Result<()> {
    let path = project_session_conversation_path(cwd, session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = ConversationFile {
        messages: messages.to_vec(),
    };
    fs::write(path, format!("{}\n", toml::to_string_pretty(&payload)?))?;
    Ok(())
}

fn persist_runtime_messages(messages: &[ChatMessage]) {
    let cwd = runtime_store().cwd.clone();
    let session_id = runtime_store().session_id.clone();
    let _ = save_session_messages(&cwd, &session_id, messages);
}

pub fn load_runtime_messages_from_file() {
    let cwd = runtime_store().cwd.clone();
    let session_id = runtime_store().session_id.clone();
    let path = project_session_conversation_path(cwd, &session_id);
    if path.exists()
        && let Ok(content) = fs::read_to_string(path)
        && let Ok(conv) = toml::from_str::<ConversationFile>(&content)
    {
        let arc = get_messages();
        let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
        *guard = conv.messages;
    }
}

pub fn runtime_messages() -> Vec<ChatMessage> {
    get_messages().lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn clear_runtime_messages() {
    let arc = get_messages();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
    persist_runtime_messages(&[]);
}

pub fn append_runtime_message(message: ChatMessage) {
    if matches!(message, ChatMessage::System { .. }) {
        return;
    }
    let arc = get_messages();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    guard.push(message);
    persist_runtime_messages(&guard);
}

static MESSAGES: OnceLock<Arc<Mutex<Vec<ChatMessage>>>> = OnceLock::new();

pub fn get_messages() -> Arc<Mutex<Vec<ChatMessage>>> {
    MESSAGES
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}
