use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use minicode_config::{
    get_active_session_context, project_session_conversation_path, project_session_dir,
    project_session_metadata_path, project_sessions_dir, project_sessions_index,
};
use minicode_types::{ChatMessage, TranscriptLine};
use serde::{Deserialize, Serialize};

// ============================================================================
// SESSION INPUT HISTORY
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct HistoryFile {
    entries: Vec<String>,
}

fn session_history_path(cwd: &Path, session_id: &str) -> std::path::PathBuf {
    project_session_dir(cwd, session_id).join("input_history.json")
}

/// 加载某个会话的历史输入并限制最多保留最近 200 条。
fn load_session_history_entries(cwd: &Path, session_id: &str) -> Vec<String> {
    let path = session_history_path(cwd, session_id);
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(parsed) = serde_json::from_str::<HistoryFile>(&content) else {
        return vec![];
    };
    let keep = 200usize;
    if parsed.entries.len() <= keep {
        return parsed.entries;
    }
    parsed.entries[parsed.entries.len() - keep..].to_vec()
}

/// 保存某个会话的历史输入并仅写入最近 200 条记录。
fn save_session_history_entries(cwd: &Path, session_id: &str, entries: &[String]) -> Result<()> {
    let path = session_history_path(cwd, session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let keep = 200usize;
    let slice = if entries.len() <= keep {
        entries.to_vec()
    } else {
        entries[entries.len() - keep..].to_vec()
    };
    let payload = HistoryFile { entries: slice };
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&payload)?),
    )?;
    Ok(())
}

/// 加载当前活动会话的历史输入。
pub fn load_history_entries() -> Vec<String> {
    let Some(ctx) = get_active_session_context() else {
        return vec![];
    };
    load_session_history_entries(&ctx.cwd, &ctx.session_id)
}

/// 保存当前活动会话的历史输入。
pub fn save_history_entries(entries: &[String]) -> Result<()> {
    let Some(ctx) = get_active_session_context() else {
        return Ok(());
    };
    save_session_history_entries(&ctx.cwd, &ctx.session_id, entries)
}

// ============================================================================
// SESSION: New session ID generation
// ============================================================================

/// Generate a unique session ID with timestamp and UUID
pub fn generate_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("sess_{:x}_{}", timestamp, uuid::Uuid::new_v4().simple())
}

// ============================================================================
// SESSION GLOBAL RUNTIME/INIT DATA
// ============================================================================

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

// ============================================================================
// DATA MODELS: Session-related structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub created_at: String, // ISO 8601
    pub ended_at: Option<String>,
    pub duration_seconds: u64,
    pub model: Option<String>,
    pub cwd: String,
    pub turn_count: usize,
    pub user_input_count: usize,
    pub tool_call_count: usize,
    #[serde(default)]
    pub status: String, // "active", "completed"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_id: String,
    pub turn_number: usize,
    pub timestamp: String,  // ISO 8601
    pub input_type: String, // "user", "tool", "system"
    pub input: String,
    #[serde(default)]
    pub tools_used: Vec<String>,
    pub duration_ms: u64,
    pub status: String, // "success", "error"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub metadata: SessionMetadata,
    pub messages: Vec<serde_json::Value>,
    pub turns: Vec<TurnRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndex {
    pub sessions: Vec<SessionIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndexEntry {
    pub session_id: String,
    pub created_at: String,
    pub ended_at: Option<String>,
    pub cwd: String,
    pub turn_count: usize,
    pub model: Option<String>,
    pub status: String,
}

// ============================================================================
// PERSISTENCE: Session save/load functions
// ============================================================================

/// Save a complete session record to disk
pub fn save_session(cwd: &Path, session: &SessionRecord) -> Result<()> {
    let session_dir = project_sessions_dir(cwd).join(&session.session_id);
    fs::create_dir_all(&session_dir)?;

    // Save metadata
    let metadata_path = project_session_metadata_path(cwd, &session.session_id);
    fs::write(
        metadata_path,
        format!("{}\n", serde_json::to_string_pretty(&session.metadata)?),
    )?;

    // Save conversation
    let conv_path = project_session_conversation_path(cwd, &session.session_id);
    let conversation = serde_json::json!({
        "messages": session.messages,
        "turns": session.turns,
    });
    fs::write(
        conv_path,
        format!("{}\n", serde_json::to_string_pretty(&conversation)?),
    )?;

    // Update session index
    update_session_index(cwd, &session.metadata)?;

    Ok(())
}

/// Update session in the index
fn update_session_index(cwd: &Path, metadata: &SessionMetadata) -> Result<()> {
    let index_path = project_sessions_index(cwd);
    let mut index: SessionIndex = if index_path.exists() {
        let content = fs::read_to_string(&index_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| SessionIndex { sessions: vec![] })
    } else {
        SessionIndex { sessions: vec![] }
    };

    // Update or insert
    if let Some(entry) = index
        .sessions
        .iter_mut()
        .find(|e| e.session_id == metadata.session_id)
    {
        entry.ended_at = metadata.ended_at.clone();
        entry.turn_count = metadata.user_input_count;
        entry.status = metadata.status.clone();
    } else {
        index.sessions.push(SessionIndexEntry {
            session_id: metadata.session_id.clone(),
            created_at: metadata.created_at.clone(),
            ended_at: metadata.ended_at.clone(),
            cwd: metadata.cwd.clone(),
            turn_count: metadata.user_input_count,
            model: metadata.model.clone(),
            status: metadata.status.clone(),
        });
    }

    // Sort by created_at descending (most recent first)
    index
        .sessions
        .sort_by(|a, b| b.created_at.cmp(&a.created_at));

    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(
        index_path,
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;

    Ok(())
}

/// Load all sessions (for resume functionality)
pub fn load_sessions(cwd: &Path) -> Result<SessionIndex> {
    let index_path = project_sessions_index(cwd);
    if !index_path.exists() {
        return Ok(SessionIndex { sessions: vec![] });
    }

    let content = fs::read_to_string(index_path)?;
    let index: SessionIndex = serde_json::from_str(&content)?;
    Ok(index)
}

/// Load a specific session for resuming
pub fn load_session(cwd: &Path, session_id: &str) -> Result<SessionRecord> {
    let metadata_path = project_session_metadata_path(cwd, session_id);
    let metadata: SessionMetadata = {
        let content = fs::read_to_string(metadata_path)?;
        serde_json::from_str(&content)?
    };

    let conv_path = project_session_conversation_path(cwd, session_id);
    let (messages, turns): (Vec<serde_json::Value>, Vec<TurnRecord>) = if conv_path.exists() {
        let content = fs::read_to_string(conv_path)?;
        let data: serde_json::Value = serde_json::from_str(&content)?;
        (
            serde_json::from_value(data.get("messages").cloned().unwrap_or_default())?,
            serde_json::from_value(data.get("turns").cloned().unwrap_or_default())?,
        )
    } else {
        (vec![], vec![])
    };

    Ok(SessionRecord {
        session_id: session_id.to_string(),
        metadata,
        messages,
        turns,
    })
}

/// Convert ChatMessage list to visible transcript lines for session recovery
pub fn render_recovered_messages(messages: &[ChatMessage]) -> Vec<TranscriptLine> {
    let mut transcript = Vec::new();

    for msg in messages {
        match msg {
            ChatMessage::System { .. } => {}
            ChatMessage::User { content } => {
                transcript.push(TranscriptLine {
                    kind: "user".to_string(),
                    body: content.clone(),
                });
            }
            ChatMessage::Assistant { content } => {
                transcript.push(TranscriptLine {
                    kind: "assistant".to_string(),
                    body: content.clone(),
                });
            }
            ChatMessage::AssistantProgress { content } => {
                transcript.push(TranscriptLine {
                    kind: "progress".to_string(),
                    body: content.clone(),
                });
            }
            ChatMessage::AssistantToolCall {
                tool_name, input, ..
            } => {
                transcript.push(TranscriptLine {
                    kind: "tool".to_string(),
                    body: format!("{}\n{}", tool_name, input),
                });
            }
            ChatMessage::ToolResult {
                content, is_error, ..
            } => {
                transcript.push(TranscriptLine {
                    kind: if *is_error { "tool:error" } else { "tool" }.to_string(),
                    body: content.clone(),
                });
            }
        }
    }

    transcript
}
