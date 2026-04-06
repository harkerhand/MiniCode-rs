mod edit_file;
mod grep_file;
mod list_file;
mod patch_file;
mod read_file;
mod write_like;
pub use edit_file::*;
pub use grep_file::*;
pub use list_file::*;
use minicode_config::runtime_store;
use minicode_permissions::get_permission_manager;
pub use patch_file::*;
pub use read_file::*;
pub use write_like::*;

use std::path::Path;
use std::{fs, path::PathBuf};

use anyhow::Result;
use minicode_tool::ToolResult;
use similar::TextDiff;

/// 生成统一 diff 文本，用于预览文件改动。
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

/// 读取目标文件，不存在时按空文件处理。
pub fn load_existing_file(target_path: impl AsRef<Path>) -> Result<String> {
    match fs::read_to_string(target_path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

/// 在通过权限审阅后将文件改动写入磁盘。
pub async fn apply_reviewed_file_change(
    file_path: &str,
    target_path: impl AsRef<Path>,
    next_content: &str,
) -> Result<ToolResult> {
    let previous = load_existing_file(target_path.as_ref())?;
    if previous == next_content {
        return Ok(ToolResult::ok(format!("No changes needed for {file_path}")));
    }

    let diff = build_unified_diff(file_path, &previous, next_content);
    let permission_manager = get_permission_manager();
    permission_manager
        .ensure_edit(target_path.as_ref().to_string_lossy().as_ref(), &diff)
        .await?;

    if let Some(parent) = target_path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target_path.as_ref(), next_content)?;

    Ok(ToolResult::ok(format!(
        "Applied reviewed changes to {file_path}"
    )))
}

/// 基于工具上下文解析目标路径，并执行权限校验。
pub async fn resolve_tool_path(target_path: &str, intent: &str) -> Result<PathBuf> {
    let cwd = runtime_store().cwd.clone();
    let base = PathBuf::from(&cwd);
    let resolved = base
        .join(target_path)
        .canonicalize()
        .unwrap_or_else(|_| base.join(target_path));

    let permission_manager = get_permission_manager();
    permission_manager
        .ensure_path_access(resolved.to_string_lossy().as_ref(), intent)
        .await?;

    Ok(resolved)
}
