use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use minicode_types::{ChatMessage, TranscriptLine};

/// Generate a unique session ID with timestamp and UUID
pub fn generate_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("sess_{:x}_{}", timestamp, uuid::Uuid::new_v4().simple())
}

static MESSAGES: LazyLock<Mutex<Vec<ChatMessage>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static TRANSCRIPT: LazyLock<Mutex<Vec<TranscriptLine>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static SESSION_ID: OnceLock<String> = OnceLock::new();
static SESSION_START_TIME: OnceLock<SystemTime> = OnceLock::new();

fn strip_system_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    messages
        .into_iter()
        .filter(|m| !matches!(m, ChatMessage::System { .. }))
        .collect()
}

pub fn initial_messages() -> Vec<ChatMessage> {
    runtime_messages()
}

pub fn set_runtime_messages(messages: Vec<ChatMessage>) {
    if let Ok(mut guard) = MESSAGES.lock() {
        *guard = strip_system_messages(messages);
    }
}

pub fn runtime_messages() -> Vec<ChatMessage> {
    if let Ok(guard) = MESSAGES.lock() {
        return guard.clone();
    }
    Vec::new()
}

pub fn clear_runtime_messages_keep_system() {
    if let Ok(mut guard) = MESSAGES.lock() {
        guard.clear();
    }
    clear_runtime_transcript();
}

pub fn initial_transcript() -> Vec<TranscriptLine> {
    runtime_transcript()
}

pub fn set_runtime_transcript(transcript: Vec<TranscriptLine>) {
    if let Ok(mut guard) = TRANSCRIPT.lock() {
        *guard = transcript;
    }
}

pub fn runtime_transcript() -> Vec<TranscriptLine> {
    if let Ok(guard) = TRANSCRIPT.lock() {
        return guard.clone();
    }
    Vec::new()
}

pub fn clear_runtime_transcript() {
    if let Ok(mut guard) = TRANSCRIPT.lock() {
        guard.clear();
    }
}

pub fn init_session_id(value: String) -> Result<()> {
    SESSION_ID
        .set(value)
        .map_err(|_| anyhow!("Session id already initialized"))
}

pub fn session_id() -> &'static String {
    SESSION_ID.get().expect("Session id not initialized")
}

pub fn init_session_start_time(value: SystemTime) -> Result<()> {
    SESSION_START_TIME
        .set(value)
        .map_err(|_| anyhow!("Session start time already initialized"))
}

pub fn session_start_time() -> SystemTime {
    *SESSION_START_TIME
        .get()
        .expect("Session start time not initialized")
}
