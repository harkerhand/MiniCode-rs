use std::fs;
use std::path::Path;

use anyhow::Result;
use minicode_config::{project_session_dir, runtime_input_history_state, runtime_store};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct HistoryFile {
    entries: Vec<String>,
}

fn session_history_path(cwd: impl AsRef<Path>, session_id: &str) -> std::path::PathBuf {
    project_session_dir(cwd, session_id).join("input_history.toml")
}

/// 加载某个会话的输入历史并限制最多保留最近 200 条。
pub fn load_history_entries() -> Vec<String> {
    let cwd = runtime_store().cwd.clone();
    let session_id = runtime_store().session_id.clone();
    let path = session_history_path(cwd, &session_id);
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(parsed) = toml::from_str::<HistoryFile>(&content) else {
        return vec![];
    };
    let keep = 200usize;
    if parsed.entries.len() <= keep {
        return parsed.entries;
    }
    parsed.entries[parsed.entries.len() - keep..].to_vec()
}

/// 保存某个会话的输入历史并仅写入最近 200 条记录。
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
    fs::write(path, format!("{}\n", toml::to_string_pretty(&payload)?))?;
    Ok(())
}

/// 新增一条历史输入并持久化到当前活动会话的 TOML 文件。
pub fn add_history_entry(entry: impl AsRef<str>) -> Result<()> {
    let history = runtime_input_history_state();
    let mut history = history.lock().unwrap_or_else(|e| e.into_inner());
    history.push(entry.as_ref().to_string());
    let cwd = runtime_store().cwd.clone();
    let session_id = runtime_store().session_id.clone();
    save_session_history_entries(cwd, &session_id, &history)?;
    Ok(())
}

/// 清空当前活动会话的历史输入文件。
pub fn clear_history_entries() -> Result<()> {
    let history = runtime_input_history_state();
    let mut history = history.lock().unwrap_or_else(|e| e.into_inner());
    history.clear();
    let cwd = runtime_store().cwd.clone();
    let session_id = runtime_store().session_id.clone();
    save_session_history_entries(cwd, &session_id, &[])
}
