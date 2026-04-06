use std::path::PathBuf;

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
