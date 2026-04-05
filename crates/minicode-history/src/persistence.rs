use std::fs;
use std::path::Path;

use anyhow::Result;
use minicode_config::{
    project_session_conversation_path, project_session_metadata_path, project_sessions_dir,
    project_sessions_index,
};
use minicode_types::ChatMessage;

use crate::{SessionIndex, SessionIndexEntry, SessionMetadata, SessionRecord};

/// Save a complete session record to disk
pub fn save_session(cwd: impl AsRef<Path>, session: &SessionRecord) -> Result<()> {
    let session_dir = project_sessions_dir(cwd.as_ref()).join(&session.session_id);
    fs::create_dir_all(&session_dir)?;

    let metadata_path = project_session_metadata_path(cwd.as_ref(), &session.session_id);
    fs::write(
        metadata_path,
        format!("{}\n", serde_json::to_string_pretty(&session.metadata)?),
    )?;

    let conv_path = project_session_conversation_path(cwd.as_ref(), &session.session_id);
    let conversation = serde_json::json!({
        "messages": session.messages,
    });
    fs::write(
        conv_path,
        format!("{}\n", serde_json::to_string_pretty(&conversation)?),
    )?;

    update_session_index(cwd.as_ref(), &session.metadata)?;
    Ok(())
}

fn update_session_index(cwd: impl AsRef<Path>, metadata: &SessionMetadata) -> Result<()> {
    let index_path = project_sessions_index(cwd.as_ref());
    let mut index: SessionIndex = if index_path.exists() {
        let content = fs::read_to_string(&index_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| SessionIndex { sessions: vec![] })
    } else {
        SessionIndex { sessions: vec![] }
    };

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
    let messages: Vec<ChatMessage> = if conv_path.exists() {
        let content = fs::read_to_string(conv_path)?;
        let data: serde_json::Value = serde_json::from_str(&content)?;
        serde_json::from_value(data.get("messages").cloned().unwrap_or_default())?
    } else {
        vec![]
    };

    Ok(SessionRecord {
        session_id: session_id.to_string(),
        metadata,
        messages,
    })
}
