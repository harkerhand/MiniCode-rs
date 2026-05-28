use std::fs;
use std::path::Path;

use std::io::BufRead;

use anyhow::Result;
use serde::Deserialize;

use crate::{SessionIndexEntry, load_sessions};
use anyhow::anyhow;
use minicode_config::{project_sessions_dir, project_sessions_index};

/// Format sessions from index as a displayable table with optional filtering
pub fn list_sessions_formatted(cwd: impl AsRef<Path>, filter_opt: Option<&str>) -> Result<String> {
    let sessions = load_sessions(cwd)?;

    if sessions.sessions.is_empty() {
        return Ok("No sessions found.".to_string());
    }

    let filtered: Vec<&SessionIndexEntry> = if let Some(filter) = filter_opt {
        let filter_lower = filter.to_lowercase();
        sessions
            .sessions
            .iter()
            .filter(|entry| {
                entry.session_id.to_lowercase().contains(&filter_lower)
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

    let mut output = String::new();
    output.push_str("Sessions:\n");
    output.push_str(&format!(
        "{:<18} {:<20} {:<20} {:<8} {:<12}\n",
        "ID", "Created", "Ended", "Turns", "Status"
    ));
    output.push_str(&"-".repeat(78));
    output.push('\n');

    for entry in filtered {
        let id_display = &entry.session_id[..entry.session_id.len().min(16)];
        let created_display = &entry.created_at[..entry.created_at.len().min(19)];
        let ended_display = entry
            .ended_at
            .as_ref()
            .map(|e| &e[..e.len().min(19)])
            .unwrap_or("—");

        output.push_str(&format!(
            "{:<18} {:<20} {:<20} {:<8} {:<12}\n",
            id_display,
            created_display,
            ended_display,
            entry.turn_count,
            entry.status
        ));
    }

    Ok(output)
}

/// Find sessions matching a prefix (for deletion and resumption)
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
    let index = load_sessions(cwd.as_ref())?;
    if !index.sessions.iter().any(|e| e.session_id == session_id) {
        return Err(anyhow!("Session not found: {}", session_id));
    }

    let session_dir = project_sessions_dir(cwd.as_ref()).join(session_id);
    if session_dir.exists() {
        fs::remove_dir_all(&session_dir)?;
    }

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
) -> Result<Option<String>> {
    let matches = find_sessions_by_prefix(cwd.as_ref(), prefix)?;

    if matches.is_empty() {
        eprintln!("✗ 未找到匹配的会话: {}", prefix);
        return Ok(None);
    }

    let sessions = load_sessions(cwd.as_ref())?;

    let target_id = if matches.len() == 1 {
        matches[0].clone()
    } else {
        eprintln!("📋 找到 {} 个匹配的会话:", matches.len());

        let items: Vec<(String, String, usize)> = matches
            .iter()
            .filter_map(|matched_id| {
                sessions
                    .sessions
                    .iter()
                    .find(|e| &e.session_id == matched_id)
                    .map(session_item_to_tuple)
            })
            .collect();

        match interactive_select(
            items,
            |idx, (id, created, turns)| {
                format!(
                    "{:<2} {:<18} {:<20} {:<6}",
                    idx,
                    &id[..id.len().min(16)],
                    created,
                    turns,
                )
            },
            &format!(
                "请选择要恢复的会话 (1-{}，或按 Enter 取消): ",
                matches.len()
            ),
        )? {
            Some((id, _, _)) => id,
            None => return Ok(None),
        }
    };

    Ok(Some(target_id))
}

fn session_item_to_tuple(entry: &SessionIndexEntry) -> (String, String, usize) {
    let created = entry.created_at.chars().take(19).collect::<String>();
    (entry.session_id.clone(), created, entry.turn_count)
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

    let stdin = std::io::stdin();
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

/// 读取 TOML 文件，不存在时返回默认值。
pub fn read_toml_file<T: for<'de> Deserialize<'de> + Default>(path: impl AsRef<Path>) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(toml::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}
