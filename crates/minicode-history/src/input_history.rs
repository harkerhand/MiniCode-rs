use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use anyhow::Result;
use minicode_config::{get_active_session_context, project_session_dir};
use minicode_types::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct HistoryFile {
    entries: Vec<String>,
}

static RUNTIME_HISTORY: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static HISTORY_LOADED: AtomicBool = AtomicBool::new(false);

fn session_history_path(cwd: impl AsRef<Path>, session_id: &str) -> std::path::PathBuf {
    project_session_dir(cwd, session_id).join("input_history.toml")
}

/// 加载某个会话的历史输入并限制最多保留最近 200 条。
fn load_session_history_entries(cwd: impl AsRef<Path>, session_id: &str) -> Vec<String> {
    let path = session_history_path(cwd, session_id);
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
    fs::write(path, format!("{}\n", toml::to_string_pretty(&payload)?))?;
    Ok(())
}

fn ensure_runtime_history_loaded() -> Vec<String> {
    if HISTORY_LOADED.load(Ordering::Relaxed) {
        return RUNTIME_HISTORY.lock().map(|g| g.clone()).unwrap_or_default();
    }
    let Some(ctx) = get_active_session_context() else {
        return vec![];
    };
    let entries = load_session_history_entries(&ctx.cwd, &ctx.session_id);
    if let Ok(mut guard) = RUNTIME_HISTORY.lock() {
        *guard = entries.clone();
    }
    HISTORY_LOADED.store(true, Ordering::Relaxed);
    entries
}

/// 加载当前活动会话的历史输入（从 runtime history 返回）。
pub fn load_history_entries() -> Vec<String> {
    ensure_runtime_history_loaded()
}

/// 新增一条历史输入并持久化当前活动会话的 runtime history。
pub fn add_history_entry(entry: &ChatMessage) -> Result<()> {
    let Some(ctx) = get_active_session_context() else {
        return Ok(());
    };
    let Some(user_input) = (match entry {
        ChatMessage::User { content } => Some(content.as_str()),
        _ => None,
    }) else {
        return Ok(());
    };
    let mut next = ensure_runtime_history_loaded();
    if next.last().map(|x| x.as_str()) != Some(user_input) {
        next.push(user_input.to_string());
    }
    save_session_history_entries(&ctx.cwd, &ctx.session_id, &next)?;
    if let Ok(mut guard) = RUNTIME_HISTORY.lock() {
        *guard = next;
    }
    HISTORY_LOADED.store(true, Ordering::Relaxed);
    Ok(())
}

/// 清空当前活动会话的历史输入文件并清空 runtime history。
pub fn clear_history_entries() -> Result<()> {
    let Some(ctx) = get_active_session_context() else {
        return Ok(());
    };
    if let Ok(mut guard) = RUNTIME_HISTORY.lock() {
        guard.clear();
    }
    HISTORY_LOADED.store(true, Ordering::Relaxed);
    save_session_history_entries(&ctx.cwd, &ctx.session_id, &[])
}
