use std::fs;
use std::path::Path;

use anyhow::Result;
use minicode_config::{get_active_session_context, project_session_dir};
use serde::{Deserialize, Serialize};

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
