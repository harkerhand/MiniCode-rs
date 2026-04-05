use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow};
use minicode_config::{project_sessions_dir, project_sessions_index};

use crate::{SessionIndexEntry, load_sessions};

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
