use std::fs;
use std::path::Path;

use anyhow::Result;
use similar::TextDiff;

use crate::tool::{ToolContext, ToolResult};

pub fn build_unified_diff(file_path: &str, before: &str, after: &str) -> String {
    if before == after {
        return format!("(no changes for {file_path})");
    }
    let diff = TextDiff::from_lines(before, after);
    diff.unified_diff()
        .context_radius(3)
        .header(&format!("a/{file_path}"), &format!("b/{file_path}"))
        .to_string()
}

pub fn load_existing_file(target_path: &Path) -> Result<String> {
    match fs::read_to_string(target_path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

pub async fn apply_reviewed_file_change(
    context: &ToolContext,
    file_path: &str,
    target_path: &Path,
    next_content: &str,
) -> Result<ToolResult> {
    let previous = load_existing_file(target_path)?;
    if previous == next_content {
        return Ok(ToolResult::ok(format!("No changes needed for {file_path}")));
    }

    let diff = build_unified_diff(file_path, &previous, next_content);
    if let Some(permissions) = &context.permissions {
        permissions
            .ensure_edit(target_path.to_string_lossy().as_ref(), &diff)
            .await?;
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target_path, next_content)?;

    Ok(ToolResult::ok(format!("Applied reviewed changes to {file_path}")))
}
