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

fn session_history_path(cwd: impl AsRef<Path>, session_id: &str) -> std::path::PathBuf {
    project_session_dir(cwd, session_id).join("input_history.json")
}

/// 加载某个会话的历史输入并限制最多保留最近 200 条。
fn load_session_history_entries(cwd: impl AsRef<Path>, session_id: &str) -> Vec<String> {
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
fn save_session_history_entries(
    cwd: impl AsRef<Path>,
    session_id: &str,
    entries: &[String],
) -> Result<()> {
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
    pub messages: Vec<ChatMessage>,
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
pub fn save_session(cwd: impl AsRef<Path>, session: &SessionRecord) -> Result<()> {
    let session_dir = project_sessions_dir(cwd.as_ref()).join(&session.session_id);
    fs::create_dir_all(&session_dir)?;

    // Save metadata
    let metadata_path = project_session_metadata_path(cwd.as_ref(), &session.session_id);
    fs::write(
        metadata_path,
        format!("{}\n", serde_json::to_string_pretty(&session.metadata)?),
    )?;

    // Save conversation
    let conv_path = project_session_conversation_path(cwd.as_ref(), &session.session_id);
    let conversation = serde_json::json!({
        "messages": session.messages,
        "turns": session.turns,
    });
    fs::write(
        conv_path,
        format!("{}\n", serde_json::to_string_pretty(&conversation)?),
    )?;

    // Update session index
    update_session_index(cwd.as_ref(), &session.metadata)?;

    Ok(())
}

/// Update session in the index
fn update_session_index(cwd: impl AsRef<Path>, metadata: &SessionMetadata) -> Result<()> {
    let index_path = project_sessions_index(cwd.as_ref());
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
pub fn load_sessions(cwd: impl AsRef<Path>) -> Result<SessionIndex> {
    let index_path = project_sessions_index(cwd.as_ref());
    if !index_path.exists() {
        return Ok(SessionIndex { sessions: vec![] });
    }

    let content = fs::read_to_string(index_path)?;
    let index: SessionIndex = serde_json::from_str(&content)?;
    Ok(index)
}

/// Load a specific session for resuming
pub fn load_session(cwd: impl AsRef<Path>, session_id: &str) -> Result<SessionRecord> {
    let metadata_path = project_session_metadata_path(cwd.as_ref(), session_id);
    let metadata: SessionMetadata = {
        let content = fs::read_to_string(metadata_path)?;
        serde_json::from_str(&content)?
    };

    let conv_path = project_session_conversation_path(cwd.as_ref(), session_id);
    let (messages, turns): (Vec<ChatMessage>, Vec<TurnRecord>) = if conv_path.exists() {
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

// ============================================================================
// HISTORY MANAGEMENT: List and delete sessions
// ============================================================================

/// Format sessions from index as a displayable table with optional filtering
pub fn list_sessions_formatted(cwd: impl AsRef<Path>, filter_opt: Option<&str>) -> Result<String> {
    let sessions = load_sessions(cwd)?;

    if sessions.sessions.is_empty() {
        return Ok("No sessions found.".to_string());
    }

    // Filter sessions if requested
    let filtered: Vec<&SessionIndexEntry> = if let Some(filter) = filter_opt {
        let filter_lower = filter.to_lowercase();
        sessions
            .sessions
            .iter()
            .filter(|entry| {
                entry.session_id.to_lowercase().contains(&filter_lower)
                    || entry
                        .model
                        .as_ref()
                        .map(|m| m.to_lowercase().contains(&filter_lower))
                        .unwrap_or(false)
            })
            .collect()
    } else {
        sessions.sessions.iter().collect()
    };

    if filtered.is_empty() {
        return Ok(format!(
            "No sessions matching filter: {}",
            filter_opt.unwrap_or("")
        ));
    }

    // Format as table
    let mut output = String::new();
    output.push_str("Sessions:\n");
    output.push_str(&format!(
        "{:<18} {:<20} {:<20} {:<8} {:<25} {:<12}\n",
        "ID", "Created", "Ended", "Turns", "Model", "Status"
    ));
    output.push_str(&"-".repeat(103));
    output.push('\n');

    for entry in filtered {
        let id_display = &entry.session_id[..entry.session_id.len().min(16)];
        let created_display = &entry.created_at[..entry.created_at.len().min(19)];
        let ended_display = entry
            .ended_at
            .as_ref()
            .map(|e| &e[..e.len().min(19)])
            .unwrap_or("—");
        let model_display = entry
            .model
            .as_ref()
            .map(|m| {
                if m.len() > 24 {
                    format!("{}...", &m[..21])
                } else {
                    m.clone()
                }
            })
            .unwrap_or_else(|| "—".to_string());

        output.push_str(&format!(
            "{:<18} {:<20} {:<20} {:<8} {:<25} {:<12}\n",
            id_display,
            created_display,
            ended_display,
            entry.turn_count,
            model_display,
            entry.status
        ));
    }

    Ok(output)
}

/// Find sessions matching a prefix (for deletion and resumption)
/// Returns list of matching session IDs
pub fn find_sessions_by_prefix(cwd: impl AsRef<Path>, prefix: &str) -> Result<Vec<String>> {
    let sessions = load_sessions(cwd)?;
    let prefix_lower = prefix.to_lowercase();

    let matching: Vec<String> = sessions
        .sessions
        .iter()
        .filter(|entry| entry.session_id.to_lowercase().starts_with(&prefix_lower))
        .map(|entry| entry.session_id.clone())
        .collect();

    Ok(matching)
}

/// Delete a session and remove its entry from the index
pub fn delete_session(cwd: impl AsRef<Path>, session_id: &str) -> Result<()> {
    // Load current index to verify session exists
    let index = load_sessions(cwd.as_ref())?;
    if !index.sessions.iter().any(|e| e.session_id == session_id) {
        return Err(anyhow!("Session not found: {}", session_id));
    }

    // Remove session directory
    let session_dir = project_sessions_dir(cwd.as_ref()).join(session_id);
    if session_dir.exists() {
        fs::remove_dir_all(&session_dir)?;
    }

    // Update index
    let mut new_index = index.clone();
    new_index.sessions.retain(|e| e.session_id != session_id);

    let index_path = project_sessions_index(cwd.as_ref());
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(
        index_path,
        format!("{}\n", serde_json::to_string_pretty(&new_index)?),
    )?;

    Ok(())
}

/// 根据前缀查询和加载会话，用于 history resume 命令
pub async fn resolve_and_load_session(
    cwd: impl AsRef<Path>,
    prefix: &str,
) -> Result<Option<(String, Vec<ChatMessage>, Vec<TranscriptLine>)>> {
    let matches = find_sessions_by_prefix(cwd.as_ref(), prefix)?;

    if matches.is_empty() {
        eprintln!("✗ 未找到匹配的会话: {}", prefix);
        return Ok(None);
    }

    let sessions = load_sessions(cwd.as_ref())?;

    let target_id = if matches.len() == 1 {
        // Single match - use directly
        matches[0].clone()
    } else {
        // Multiple matches - interactive selection
        eprintln!("📋 找到 {} 个匹配的会话:", matches.len());

        let items: Vec<(String, String, usize, String)> = matches
            .iter()
            .filter_map(|matched_id| {
                sessions
                    .sessions
                    .iter()
                    .find(|e| &e.session_id == matched_id)
                    .map(|entry| {
                        let created = entry.created_at.chars().take(19).collect::<String>();
                        let model = entry.model.as_deref().unwrap_or("—").to_string();
                        (matched_id.clone(), created, entry.turn_count, model)
                    })
            })
            .collect();

        match interactive_select(
            items,
            |idx, (id, created, turns, model)| {
                format!(
                    "{:<2} {:<18} {:<20} {:<6} {:<30}",
                    idx,
                    &id[..id.len().min(16)],
                    created,
                    turns,
                    model
                )
            },
            &format!(
                "请选择要恢复的会话 (1-{}，或按 Enter 取消): ",
                matches.len()
            ),
        )? {
            Some((id, _, _, _)) => id,
            None => return Ok(None),
        }
    };

    // Load session data
    match load_session(cwd.as_ref(), &target_id) {
        Ok(session) => {
            eprintln!("✨ 正在加载会话数据...\n");

            let recovered_messages: Vec<ChatMessage> = session.messages;

            let transcript_lines = render_recovered_messages(&recovered_messages);
            let transcript = transcript_lines
                .into_iter()
                .map(|line| TranscriptLine {
                    kind: line.kind,
                    body: line.body,
                })
                .collect();

            Ok(Some((target_id, recovered_messages, transcript)))
        }
        Err(e) => {
            eprintln!("⚠️  无法加载会话: {}", e);
            Ok(None)
        }
    }
}

/// 通用的交互式列表选择函数
pub fn interactive_select<T: Clone>(
    items: Vec<T>,
    format_fn: impl Fn(usize, &T) -> String,
    prompt: &str,
) -> Result<Option<T>> {
    if items.is_empty() {
        return Ok(None);
    }

    eprintln!();
    for (idx, item) in items.iter().enumerate() {
        eprintln!("{}", format_fn(idx + 1, item));
    }

    eprintln!();
    eprint!("{}", prompt);
    use std::io::{self, BufRead};

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;

    let line = line.trim();
    if line.is_empty() {
        eprintln!("已取消。");
        return Ok(None);
    }

    match line.parse::<usize>() {
        Ok(choice) if choice > 0 && choice <= items.len() => Ok(Some(items[choice - 1].clone())),
        _ => {
            eprintln!("✗ 无效的选择。");
            Ok(None)
        }
    }
}
