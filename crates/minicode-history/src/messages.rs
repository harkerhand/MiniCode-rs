use std::fs;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use minicode_config::{project_session_conversation_path, runtime_store};
use minicode_types::ChatMessage;
use serde::{Deserialize, Serialize};

use crate::read_toml_file;

/// Generate a unique session ID with timestamp and UUID
pub fn generate_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("sess_{:x}_{}", timestamp, uuid::Uuid::new_v4().simple())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConversationFile {
    messages: Vec<ChatMessage>,
}

fn save_session_messages(
    cwd: impl AsRef<Path>,
    session_id: &str,
    messages: &[ChatMessage],
) -> Result<()> {
    if messages.is_empty() {
        return Ok(()); // 不保存没有任何用户输入的会话
    }
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

pub fn load_runtime_messages_from_file() -> Vec<ChatMessage> {
    let cwd = runtime_store().cwd.clone();
    let session_id = runtime_store().session_id.clone();
    let path = project_session_conversation_path(cwd, &session_id);
    let messages: ConversationFile = read_toml_file(path).unwrap_or_default();
    messages.messages
}

pub fn runtime_messages() -> Vec<ChatMessage> {
    get_messages().lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn runtime_messages_for_context() -> Vec<ChatMessage> {
    runtime_messages()
        .into_iter()
        .filter(ChatMessage::should_include_in_context)
        .collect()
}

pub fn runtime_messages_count() -> usize {
    get_messages().lock().map(|g| g.len()).unwrap_or_default()
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
    let persisted = guard
        .iter()
        .filter(|msg| msg.should_record())
        .cloned()
        .collect::<Vec<_>>();
    persist_runtime_messages(&persisted);
}

static MESSAGES: LazyLock<Arc<Mutex<Vec<ChatMessage>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(load_runtime_messages_from_file())));

pub fn get_messages() -> Arc<Mutex<Vec<ChatMessage>>> {
    MESSAGES.clone()
}
