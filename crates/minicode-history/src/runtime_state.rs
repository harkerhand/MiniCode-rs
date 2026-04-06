use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use minicode_config::{project_session_dir, runtime_messages_state, runtime_store};
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

fn session_conversation_path(cwd: impl AsRef<Path>, session_id: &str) -> std::path::PathBuf {
    project_session_dir(cwd, session_id).join("conversation.toml")
}

fn save_session_messages(
    cwd: impl AsRef<Path>,
    session_id: &str,
    messages: &[ChatMessage],
) -> Result<()> {
    let path = session_conversation_path(cwd, session_id);
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

pub fn set_runtime_messages(messages: Vec<ChatMessage>) {
    let arc = runtime_messages_state();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    *guard = messages.clone();
    persist_runtime_messages(&guard);
}

pub fn runtime_messages() -> Vec<ChatMessage> {
    runtime_messages_state()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn clear_runtime_messages() {
    let arc = runtime_messages_state();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
    persist_runtime_messages(&[]);
}

pub fn append_runtime_message(message: ChatMessage) {
    if matches!(message, ChatMessage::System { .. }) {
        return;
    }
    let arc = runtime_messages_state();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    guard.push(message);
    persist_runtime_messages(&guard);
}
