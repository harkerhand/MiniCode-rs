use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use minicode_config::{
    project_session_metadata_path, project_sessions_dir, project_sessions_index, runtime_store,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub created_at: String,
    pub ended_at: Option<String>,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub turn_count: usize,
    pub user_input_count: usize,
    pub tool_call_count: usize,
    #[serde(default)]
    pub status: String,
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
    pub cwd: PathBuf,
    pub turn_count: usize,
    pub model: Option<String>,
    pub status: String,
}

/// 把完整的会话记录保存到磁盘上，供后续恢复使用
pub fn save_session_metadata(session: &SessionMetadata) -> Result<()> {
    if session.turn_count == 0 {
        return Ok(()); // 不保存没有任何用户输入的会话
    }
    let cwd = runtime_store().cwd.clone();
    let session_dir = project_sessions_dir(&cwd).join(&session.session_id);
    fs::create_dir_all(&session_dir)?;

    let metadata_path = project_session_metadata_path(&cwd, &session.session_id);
    fs::write(
        metadata_path,
        format!("{}\n", serde_json::to_string_pretty(&session)?),
    )?;

    update_session_index(&cwd, session)?;
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
pub fn check_session(cwd: impl AsRef<Path>, session_id: &str) -> Result<()> {
    let metadata_path = project_session_metadata_path(cwd.as_ref(), session_id);
    let content = fs::read_to_string(metadata_path)?;
    let _metadata: SessionMetadata = serde_json::from_str(&content)?;
    Ok(())
}
