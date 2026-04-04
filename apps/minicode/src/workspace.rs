use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::tool::ToolContext;

pub async fn resolve_tool_path(
    context: &ToolContext,
    target_path: &str,
    intent: &str,
) -> Result<PathBuf> {
    let base = PathBuf::from(&context.cwd);
    let resolved = base
        .join(target_path)
        .canonicalize()
        .unwrap_or_else(|_| base.join(target_path));

    if context.permissions.is_none() {
        ensure_inside_workspace(&base, &resolved)?;
        return Ok(resolved);
    }

    if let Some(permissions) = &context.permissions {
        permissions
            .ensure_path_access(resolved.to_string_lossy().as_ref(), intent)
            .await?;
    }

    Ok(resolved)
}

fn ensure_inside_workspace(root: &Path, resolved: &Path) -> Result<()> {
    let Ok(relative) = resolved.strip_prefix(root) else {
        return Err(anyhow!("Path escapes workspace: {}", resolved.display()));
    };
    if relative
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(anyhow!("Path escapes workspace: {}", resolved.display()));
    }
    Ok(())
}
