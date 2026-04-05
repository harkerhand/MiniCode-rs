use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use minicode_prompt::SkillSummary;

#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub summary: SkillSummary,
    pub content: String,
}

/// 从 SKILL.md 中提取首段可读描述。
fn extract_description(markdown: &str) -> String {
    let normalized = markdown.replace("\r\n", "\n");
    for block in normalized.split("\n\n") {
        let b = block.trim();
        if b.is_empty() || b.starts_with('#') {
            continue;
        }
        if let Some(line) = b
            .lines()
            .map(str::trim)
            .find(|x| !x.is_empty() && !x.starts_with('#'))
        {
            return line.replace('`', "");
        }
    }
    "No description provided.".to_string()
}

/// 返回技能搜索根目录及其来源标签。
fn skill_roots(cwd: impl AsRef<Path>) -> Vec<(PathBuf, String)> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    vec![
        (
            cwd.as_ref().join(".mini-code/skills"),
            "project".to_string(),
        ),
        (home.join(".mini-code/skills"), "user".to_string()),
        (
            cwd.as_ref().join(".claude/skills"),
            "compat_project".to_string(),
        ),
        (home.join(".claude/skills"), "compat_user".to_string()),
    ]
}

/// 扫描并返回当前可用技能摘要。
pub fn discover_skills(cwd: impl AsRef<Path>) -> Vec<SkillSummary> {
    let mut by_name: HashMap<String, SkillSummary> = HashMap::new();
    for (root, source) in skill_roots(cwd) {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if by_name.contains_key(&name) {
                continue;
            }
            let skill_path = entry.path().join("SKILL.md");
            let Ok(content) = fs::read_to_string(&skill_path) else {
                continue;
            };
            by_name.insert(
                name.clone(),
                SkillSummary {
                    name,
                    description: extract_description(&content),
                    path: skill_path.to_string_lossy().to_string(),
                    source: source.clone(),
                },
            );
        }
    }

    by_name.into_values().collect()
}

/// 按名称加载技能全文与元信息。
pub fn load_skill(cwd: impl AsRef<Path>, name: &str) -> Option<LoadedSkill> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return None;
    }
    for (root, source) in skill_roots(cwd) {
        let p = root.join(normalized).join("SKILL.md");
        let Ok(content) = fs::read_to_string(&p) else {
            continue;
        };
        let summary = SkillSummary {
            name: normalized.to_string(),
            description: extract_description(&content),
            path: p.to_string_lossy().to_string(),
            source,
        };
        return Some(LoadedSkill { summary, content });
    }
    None
}

/// 将外部技能安装到用户或项目作用域。
pub fn install_skill(
    cwd: impl AsRef<Path>,
    source_path: &str,
    name: Option<String>,
    project: bool,
) -> Result<(String, String)> {
    let source = cwd.as_ref().join(source_path);
    let (content, inferred_name) = if source.is_dir() {
        let skill_file = source.join("SKILL.md");
        (
            fs::read_to_string(&skill_file)?,
            source
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    } else {
        let skill_file = if source.ends_with("SKILL.md") {
            source.clone()
        } else {
            source.with_file_name("SKILL.md")
        };
        let name = skill_file
            .parent()
            .and_then(|p| p.file_name())
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        (fs::read_to_string(&skill_file)?, name)
    };

    let final_name = name.unwrap_or(inferred_name).trim().to_string();
    if final_name.is_empty() {
        return Err(anyhow!("Skill name cannot be empty."));
    }

    let root = if project {
        cwd.as_ref().join(".mini-code/skills")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".mini-code/skills")
    };

    let target_dir = root.join(&final_name);
    fs::create_dir_all(&target_dir)?;
    let target = target_dir.join("SKILL.md");
    fs::write(&target, content)?;

    Ok((final_name, target.to_string_lossy().to_string()))
}

/// 删除托管技能目录并返回删除结果。
pub fn remove_managed_skill(
    cwd: impl AsRef<Path>,
    name: &str,
    project: bool,
) -> Result<(bool, String)> {
    let root = if project {
        cwd.as_ref().join(".mini-code/skills")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".mini-code/skills")
    };
    let target = root.join(name);
    let target_str = target.to_string_lossy().to_string();
    match fs::remove_dir_all(&target) {
        Ok(_) => Ok((true, target_str)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((false, target_str)),
        Err(err) => Err(err.into()),
    }
}
