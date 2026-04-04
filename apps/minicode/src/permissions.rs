use std::collections::HashSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::mini_code_permissions_path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PermissionStore {
    #[serde(default)]
    allowed_directory_prefixes: Vec<String>,
    #[serde(default)]
    denied_directory_prefixes: Vec<String>,
    #[serde(default)]
    allowed_command_patterns: Vec<String>,
    #[serde(default)]
    denied_command_patterns: Vec<String>,
    #[serde(default)]
    allowed_edit_patterns: Vec<String>,
    #[serde(default)]
    denied_edit_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PermissionManager {
    workspace_root: PathBuf,
    allowed_directory_prefixes: HashSet<String>,
    denied_directory_prefixes: HashSet<String>,
    session_allowed_paths: HashSet<String>,
    session_denied_paths: HashSet<String>,
    allowed_command_patterns: HashSet<String>,
    denied_command_patterns: HashSet<String>,
    session_allowed_commands: HashSet<String>,
    session_denied_commands: HashSet<String>,
    allowed_edit_patterns: HashSet<String>,
    denied_edit_patterns: HashSet<String>,
    session_allowed_edits: HashSet<String>,
    session_denied_edits: HashSet<String>,
    turn_allowed_edits: HashSet<String>,
    turn_allow_all_edits: bool,
}

impl PermissionManager {
    pub fn new(workspace_root: PathBuf) -> Result<Self> {
        let store = read_store()?;
        Ok(Self {
            workspace_root,
            allowed_directory_prefixes: store.allowed_directory_prefixes.into_iter().collect(),
            denied_directory_prefixes: store.denied_directory_prefixes.into_iter().collect(),
            session_allowed_paths: HashSet::new(),
            session_denied_paths: HashSet::new(),
            allowed_command_patterns: store.allowed_command_patterns.into_iter().collect(),
            denied_command_patterns: store.denied_command_patterns.into_iter().collect(),
            session_allowed_commands: HashSet::new(),
            session_denied_commands: HashSet::new(),
            allowed_edit_patterns: store.allowed_edit_patterns.into_iter().collect(),
            denied_edit_patterns: store.denied_edit_patterns.into_iter().collect(),
            session_allowed_edits: HashSet::new(),
            session_denied_edits: HashSet::new(),
            turn_allowed_edits: HashSet::new(),
            turn_allow_all_edits: false,
        })
    }

    pub fn begin_turn(&mut self) {
        self.turn_allowed_edits.clear();
        self.turn_allow_all_edits = false;
    }

    pub fn end_turn(&mut self) {
        self.turn_allowed_edits.clear();
        self.turn_allow_all_edits = false;
    }

    pub fn get_summary(&self) -> Vec<String> {
        let mut summary = vec![format!("cwd: {}", self.workspace_root.display())];
        if self.allowed_directory_prefixes.is_empty() {
            summary.push("extra allowed dirs: none".to_string());
        } else {
            summary.push(format!(
                "extra allowed dirs: {}",
                self.allowed_directory_prefixes
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.allowed_command_patterns.is_empty() {
            summary.push("dangerous allowlist: none".to_string());
        } else {
            summary.push(format!(
                "dangerous allowlist: {}",
                self.allowed_command_patterns
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        summary
    }

    pub async fn ensure_path_access(&self, target_path: &str, _intent: &str) -> Result<()> {
        let normalized = std::path::Path::new(target_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(target_path));

        if normalized.starts_with(&self.workspace_root) {
            return Ok(());
        }

        let target = normalized.to_string_lossy().to_string();
        if self.session_denied_paths.contains(&target)
            || self
                .denied_directory_prefixes
                .iter()
                .any(|x| target.starts_with(x))
        {
            return Err(anyhow!("Access denied for path outside cwd: {target}"));
        }

        if self.session_allowed_paths.contains(&target)
            || self
                .allowed_directory_prefixes
                .iter()
                .any(|x| target.starts_with(x))
        {
            return Ok(());
        }

        if Self::confirm(&format!(
            "允许访问工作区外路径吗？\n- cwd: {}\n- target: {}\n输入 y 允许，其他键拒绝: ",
            self.workspace_root.display(),
            target
        ))? {
            return Ok(());
        }

        Err(anyhow!("Access denied for path outside cwd: {target}"))
    }

    pub async fn ensure_command(
        &self,
        command: &str,
        args: &[String],
        command_cwd: &str,
    ) -> Result<()> {
        self.ensure_path_access(command_cwd, "command_cwd").await?;
        let signature = format!("{} {}", command, args.join(" ")).trim().to_string();

        let dangerous = classify_dangerous_command(command, args);
        if dangerous.is_none() {
            return Ok(());
        }

        if self.session_denied_commands.contains(&signature)
            || self.denied_command_patterns.contains(&signature)
        {
            return Err(anyhow!("Command denied: {signature}"));
        }
        if self.session_allowed_commands.contains(&signature)
            || self.allowed_command_patterns.contains(&signature)
        {
            return Ok(());
        }

        if Self::confirm(&format!(
            "检测到高风险命令，是否允许执行？\n- command: {}\n- reason: {}\n输入 y 允许，其他键拒绝: ",
            signature,
            dangerous.unwrap_or_default()
        ))? {
            return Ok(());
        }

        Err(anyhow!("Command denied: {signature}"))
    }

    pub async fn ensure_edit(&self, target_path: &str, _diff_preview: &str) -> Result<()> {
        if self.session_denied_edits.contains(target_path)
            || self.denied_edit_patterns.contains(target_path)
        {
            return Err(anyhow!("Edit denied: {target_path}"));
        }

        if self.turn_allow_all_edits
            || self.session_allowed_edits.contains(target_path)
            || self.turn_allowed_edits.contains(target_path)
            || self.allowed_edit_patterns.contains(target_path)
        {
            return Ok(());
        }

        if Self::confirm(&format!(
            "允许修改文件吗？\n- file: {}\n输入 y 允许，其他键拒绝。\n",
            target_path
        ))? {
            return Ok(());
        }

        Err(anyhow!("Edit denied: {target_path}"))
    }

    pub fn persist(&self) -> Result<()> {
        let path = mini_code_permissions_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let store = PermissionStore {
            allowed_directory_prefixes: self.allowed_directory_prefixes.iter().cloned().collect(),
            denied_directory_prefixes: self.denied_directory_prefixes.iter().cloned().collect(),
            allowed_command_patterns: self.allowed_command_patterns.iter().cloned().collect(),
            denied_command_patterns: self.denied_command_patterns.iter().cloned().collect(),
            allowed_edit_patterns: self.allowed_edit_patterns.iter().cloned().collect(),
            denied_edit_patterns: self.denied_edit_patterns.iter().cloned().collect(),
        };
        fs::write(path, format!("{}\n", serde_json::to_string_pretty(&store)?))?;
        Ok(())
    }
}

impl PermissionManager {
    fn confirm(prompt: &str) -> Result<bool> {
        if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
            return Ok(false);
        }

        let mut stdout = io::stdout();
        write!(stdout, "{}", prompt)?;
        stdout.flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES"))
    }
}

fn read_store() -> Result<PermissionStore> {
    let path = mini_code_permissions_path();
    match fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(PermissionStore::default()),
        Err(err) => Err(err.into()),
    }
}

fn classify_dangerous_command(command: &str, args: &[String]) -> Option<String> {
    let signature = format!("{} {}", command, args.join(" ")).trim().to_string();
    if command == "git" {
        if args.iter().any(|x| x == "reset") && args.iter().any(|x| x == "--hard") {
            return Some(format!(
                "git reset --hard can discard local changes ({signature})"
            ));
        }
        if args.iter().any(|x| x == "clean") {
            return Some(format!(
                "git clean can delete untracked files ({signature})"
            ));
        }
        if args.iter().any(|x| x == "push") && args.iter().any(|x| x == "--force" || x == "-f") {
            return Some(format!(
                "git push --force rewrites remote history ({signature})"
            ));
        }
    }
    if command == "npm" && args.iter().any(|x| x == "publish") {
        return Some(format!("npm publish affects remote registry ({signature})"));
    }
    if matches!(command, "node" | "python3" | "bash" | "sh" | "bun") {
        return Some(format!(
            "{command} can execute arbitrary code ({signature})"
        ));
    }
    None
}
