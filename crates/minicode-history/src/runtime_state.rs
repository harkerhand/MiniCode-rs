use std::sync::OnceLock;
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

static INITIAL_MESSAGES: OnceLock<Vec<ChatMessage>> = OnceLock::new();
static INITIAL_TRANSCRIPT: OnceLock<Vec<TranscriptLine>> = OnceLock::new();
static SESSION_ID: OnceLock<String> = OnceLock::new();
static SESSION_START_TIME: OnceLock<SystemTime> = OnceLock::new();

pub fn init_initial_messages(messages: Vec<ChatMessage>) -> Result<()> {
    INITIAL_MESSAGES
        .set(messages)
        .map_err(|_| anyhow!("Initial messages already initialized"))
}

pub fn initial_messages() -> &'static Vec<ChatMessage> {
    INITIAL_MESSAGES
        .get()
        .expect("Initial messages not initialized")
}

pub fn init_initial_transcript(transcript: Vec<TranscriptLine>) -> Result<()> {
    INITIAL_TRANSCRIPT
        .set(transcript)
        .map_err(|_| anyhow!("Initial transcript already initialized"))
}

pub fn initial_transcript() -> &'static Vec<TranscriptLine> {
    INITIAL_TRANSCRIPT
        .get()
        .expect("Initial transcript not initialized")
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
