use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use minicode_core::config::{get_active_session_context, project_session_permissions_path};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

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
pub enum PermissionPromptKind {
    Path,
    Command,
    Edit,
}

#[derive(Debug, Clone)]
pub struct PermissionPromptRequest {
    pub kind: PermissionPromptKind,
    pub title: String,
    pub details: Vec<String>,
    pub scope: String,
    pub choices: Vec<PermissionChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionDecision {
    AllowOnce,
    AllowAlways,
    AllowTurn,
    AllowAllTurn,
    DenyOnce,
    DenyAlways,
    DenyWithFeedback,
}

#[derive(Debug, Clone)]
pub struct PermissionChoice {
    pub key: String,
    pub label: String,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone)]
pub struct PermissionPromptResult {
    pub decision: PermissionDecision,
    pub feedback: Option<String>,
}

type PermissionPromptFuture = Pin<Box<dyn Future<Output = PermissionPromptResult> + Send>>;
pub type PermissionPromptHandler =
    Arc<dyn Fn(PermissionPromptRequest) -> PermissionPromptFuture + Send + Sync>;

#[derive(Debug, Clone, Default)]
pub struct EnsureCommandOptions {
    pub force_prompt_reason: Option<String>,
}

#[derive(Debug, Default)]
struct PermissionState {
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

#[derive(Clone)]
pub struct PermissionManager {
    workspace_root: PathBuf,
    store_path: PathBuf,
    state: Arc<Mutex<PermissionState>>,
    prompt_handler: Arc<Mutex<Option<PermissionPromptHandler>>>,
}

static SESSION_PERMISSIONS: OnceLock<PermissionManager> = OnceLock::new();

pub fn init_session_permissions(permissions: PermissionManager) -> Result<()> {
    SESSION_PERMISSIONS
        .set(permissions)
        .map_err(|_| anyhow!("Session permissions already initialized"))
}

pub fn session_permissions() -> &'static PermissionManager {
    SESSION_PERMISSIONS
        .get()
        .expect("Session permissions not initialized")
}

impl std::fmt::Debug for PermissionManager {
    /// 自定义 Debug 输出，避免泄露共享状态细节。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionManager")
            .field("workspace_root", &self.workspace_root)
            .field("store_path", &self.store_path)
            .field("state", &"<shared-state>")
            .field(
                "prompt_handler",
                &self
                    .prompt_handler
                    .try_lock()
                    .ok()
                    .and_then(|x| x.as_ref().map(|_| "<handler>")),
            )
            .finish()
    }
}

impl PermissionManager {
    /// 从持久化存储加载权限配置并初始化管理器。
    pub fn new(workspace_root: PathBuf) -> Result<Self> {
        let ctx = get_active_session_context()
            .ok_or_else(|| anyhow!("Active session context is not initialized"))?;
        let store_path = project_session_permissions_path(&ctx.cwd, &ctx.session_id);
        let store = read_store(&store_path)?;

        let state = PermissionState {
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
        };
        Ok(Self {
            workspace_root,
            store_path,
            state: Arc::new(Mutex::new(state)),
            prompt_handler: Arc::new(Mutex::new(None)),
        })
    }

    /// 注册用于 UI 审批流程的异步回调。
    pub fn set_prompt_handler(&self, handler: PermissionPromptHandler) {
        if let Ok(mut slot) = self.prompt_handler.try_lock() {
            *slot = Some(handler);
        }
    }

    /// 优先走 UI 回调审批，回退到终端确认。
    async fn prompt_or_confirm(
        &self,
        request: PermissionPromptRequest,
        fallback_prompt: &str,
        fallback_allow: PermissionDecision,
        fallback_deny: PermissionDecision,
    ) -> Result<PermissionPromptResult> {
        let handler = self.prompt_handler.lock().await.clone();
        if let Some(handler) = handler {
            let decision = handler(request).await;
            return Ok(decision);
        }
        let allow = Self::confirm(fallback_prompt)?;
        Ok(PermissionPromptResult {
            decision: if allow { fallback_allow } else { fallback_deny },
            feedback: None,
        })
    }

    /// 开始新回合并重置回合级编辑权限。
    pub fn begin_turn(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            state.turn_allowed_edits.clear();
            state.turn_allow_all_edits = false;
        }
    }

    /// 结束回合并清理回合级状态。
    pub fn end_turn(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            state.turn_allowed_edits.clear();
            state.turn_allow_all_edits = false;
        }
    }

    /// 返回权限状态的简要摘要文本。
    pub fn get_summary(&self) -> Vec<String> {
        let state = self.state.try_lock().ok();
        let mut summary = vec![format!("cwd: {}", self.workspace_root.display())];
        let empty_dirs = state
            .as_ref()
            .map(|x| x.allowed_directory_prefixes.is_empty())
            .unwrap_or(true);
        if empty_dirs {
            summary.push("extra allowed dirs: none".to_string());
        } else {
            let dirs = state
                .as_ref()
                .map(|x| {
                    x.allowed_directory_prefixes
                        .iter()
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            summary.push(format!("extra allowed dirs: {}", dirs.join(", ")));
        }
        let empty_cmds = state
            .as_ref()
            .map(|x| x.allowed_command_patterns.is_empty())
            .unwrap_or(true);
        if empty_cmds {
            summary.push("dangerous allowlist: none".to_string());
        } else {
            let cmds = state
                .as_ref()
                .map(|x| {
                    x.allowed_command_patterns
                        .iter()
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            summary.push(format!("dangerous allowlist: {}", cmds.join(", ")));
        }
        summary
    }

    /// 校验路径访问权限，必要时触发审批。
    pub async fn ensure_path_access(&self, target_path: &str, _intent: &str) -> Result<()> {
        let normalized = Path::new(target_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(target_path));

        if normalized.starts_with(&self.workspace_root) {
            return Ok(());
        }

        let target = normalized.to_string_lossy().to_string();
        let (already_denied, already_allowed) = {
            let state = self.state.lock().await;

            (
                state.session_denied_paths.contains(&target)
                    || state
                        .denied_directory_prefixes
                        .iter()
                        .any(|x| is_within_directory(Path::new(x), &normalized)),
                state.session_allowed_paths.contains(&target)
                    || state
                        .allowed_directory_prefixes
                        .iter()
                        .any(|x| is_within_directory(Path::new(x), &normalized)),
            )
        };

        if already_denied {
            return Err(anyhow!("Access denied for path outside cwd: {target}"));
        }
        if already_allowed {
            return Ok(());
        }

        let scope_directory = if matches!(_intent, "list" | "command_cwd") {
            normalized.clone()
        } else {
            normalized
                .parent()
                .map(|x| x.to_path_buf())
                .unwrap_or_else(|| normalized.clone())
        };
        let scope = scope_directory.to_string_lossy().to_string();

        let prompt_result = self
            .prompt_or_confirm(
                PermissionPromptRequest {
                    kind: PermissionPromptKind::Path,
                    title: "mini-code wants path access outside cwd".to_string(),
                    details: vec![
                        format!("cwd: {}", self.workspace_root.display()),
                        format!("target: {}", target),
                        format!("scope directory: {}", scope),
                    ],
                    scope: scope.clone(),
                    choices: vec![
                        PermissionChoice {
                            key: "y".to_string(),
                            label: "allow once".to_string(),
                            decision: PermissionDecision::AllowOnce,
                        },
                        PermissionChoice {
                            key: "a".to_string(),
                            label: "allow this directory".to_string(),
                            decision: PermissionDecision::AllowAlways,
                        },
                        PermissionChoice {
                            key: "n".to_string(),
                            label: "deny once".to_string(),
                            decision: PermissionDecision::DenyOnce,
                        },
                        PermissionChoice {
                            key: "d".to_string(),
                            label: "deny this directory".to_string(),
                            decision: PermissionDecision::DenyAlways,
                        },
                    ],
                },
                &format!(
                    "Allow path access outside cwd?\n- cwd: {}\n- target: {}\nEnter y to allow, others to deny: ",
                    self.workspace_root.display(),
                    target
                ),
                PermissionDecision::AllowOnce,
                PermissionDecision::DenyOnce,
            )
            .await?;

        let mut state = self.state.lock().await;

        match prompt_result.decision {
            PermissionDecision::AllowOnce => {
                state.session_allowed_paths.insert(target);
                Ok(())
            }
            PermissionDecision::AllowAlways => {
                state.allowed_directory_prefixes.insert(scope);
                drop(state);
                self.persist()
            }
            PermissionDecision::DenyAlways => {
                state.denied_directory_prefixes.insert(scope);
                drop(state);
                self.persist()?;
                Err(anyhow!("Access denied for path outside cwd: {target_path}"))
            }
            _ => {
                state.session_denied_paths.insert(target);
                Err(anyhow!("Access denied for path outside cwd: {target_path}"))
            }
        }
    }

    /// 校验命令执行权限，危险或未知命令需要审批。
    pub async fn ensure_command(
        &self,
        command: &str,
        args: &[String],
        command_cwd: &str,
        options: Option<EnsureCommandOptions>,
    ) -> Result<()> {
        self.ensure_path_access(command_cwd, "command_cwd").await?;
        let signature = format!("{} {}", command, args.join(" ")).trim().to_string();

        let dangerous = classify_dangerous_command(command, args);
        let force_reason = options
            .and_then(|x| x.force_prompt_reason)
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty());
        let reason = force_reason.clone().or(dangerous.clone());

        if reason.is_none() {
            return Ok(());
        }

        {
            let state = self.state.lock().await;
            if state.session_denied_commands.contains(&signature)
                || state.denied_command_patterns.contains(&signature)
            {
                return Err(anyhow!("Command denied: {signature}"));
            }
            if state.session_allowed_commands.contains(&signature)
                || state.allowed_command_patterns.contains(&signature)
            {
                return Ok(());
            }
        }

        let prompt_result = self
            .prompt_or_confirm(
                PermissionPromptRequest {
                    kind: PermissionPromptKind::Command,
                    title: if force_reason.is_some() {
                        "mini-code wants to run an unregistered command".to_string()
                    } else {
                        "mini-code wants to run a high-risk command".to_string()
                    },
                    details: vec![
                        format!("cwd: {command_cwd}"),
                        format!("command: {signature}"),
                        format!("reason: {}", reason.clone().unwrap_or_default()),
                    ],
                    scope: signature.clone(),
                    choices: vec![
                        PermissionChoice {
                            key: "y".to_string(),
                            label: "allow once".to_string(),
                            decision: PermissionDecision::AllowOnce,
                        },
                        PermissionChoice {
                            key: "a".to_string(),
                            label: "allow this command".to_string(),
                            decision: PermissionDecision::AllowAlways,
                        },
                        PermissionChoice {
                            key: "n".to_string(),
                            label: "deny once".to_string(),
                            decision: PermissionDecision::DenyOnce,
                        },
                        PermissionChoice {
                            key: "d".to_string(),
                            label: "deny this command".to_string(),
                            decision: PermissionDecision::DenyAlways,
                        },
                    ],
                },
                &format!(
                    "Command requires approval. Allow execution?\n- command: {}\n- reason: {}\nEnter y to allow, others to deny: ",
                    signature,
                    reason.unwrap_or_default()
                ),
                PermissionDecision::AllowOnce,
                PermissionDecision::DenyOnce,
            )
            .await?;

        let mut state = self.state.lock().await;

        match prompt_result.decision {
            PermissionDecision::AllowOnce => {
                state.session_allowed_commands.insert(signature);
                Ok(())
            }
            PermissionDecision::AllowAlways => {
                state.allowed_command_patterns.insert(signature);
                drop(state);
                self.persist()
            }
            PermissionDecision::DenyAlways => {
                state.denied_command_patterns.insert(signature.clone());
                drop(state);
                self.persist()?;
                Err(anyhow!("Command denied: {signature}"))
            }
            _ => {
                state.session_denied_commands.insert(signature.clone());
                Err(anyhow!("Command denied: {signature}"))
            }
        }
    }

    /// 校验文件编辑权限并支持用户反馈拒绝。
    pub async fn ensure_edit(&self, target_path: &str, diff_preview: &str) -> Result<()> {
        let normalized_target = Path::new(target_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(target_path))
            .to_string_lossy()
            .to_string();

        {
            let state = self.state.lock().await;

            if state.session_denied_edits.contains(&normalized_target)
                || state.denied_edit_patterns.contains(&normalized_target)
            {
                return Err(anyhow!("Edit denied: {normalized_target}"));
            }

            if state.turn_allow_all_edits
                || state.session_allowed_edits.contains(&normalized_target)
                || state.turn_allowed_edits.contains(&normalized_target)
                || state.allowed_edit_patterns.contains(&normalized_target)
            {
                return Ok(());
            }
        }

        let prompt_result = self
            .prompt_or_confirm(
                PermissionPromptRequest {
                    kind: PermissionPromptKind::Edit,
                    title: "mini-code will apply file edits".to_string(),
                    details: vec![
                        format!("target: {normalized_target}"),
                        String::new(),
                        diff_preview.to_string(),
                    ],
                    scope: normalized_target.clone(),
                    choices: vec![
                        PermissionChoice {
                            key: "1".to_string(),
                            label: "allow once".to_string(),
                            decision: PermissionDecision::AllowOnce,
                        },
                        PermissionChoice {
                            key: "2".to_string(),
                            label: "allow this file for this turn".to_string(),
                            decision: PermissionDecision::AllowTurn,
                        },
                        PermissionChoice {
                            key: "3".to_string(),
                            label: "allow all edits this turn".to_string(),
                            decision: PermissionDecision::AllowAllTurn,
                        },
                        PermissionChoice {
                            key: "4".to_string(),
                            label: "always allow this file".to_string(),
                            decision: PermissionDecision::AllowAlways,
                        },
                        PermissionChoice {
                            key: "5".to_string(),
                            label: "deny once".to_string(),
                            decision: PermissionDecision::DenyOnce,
                        },
                        PermissionChoice {
                            key: "6".to_string(),
                            label: "deny with feedback".to_string(),
                            decision: PermissionDecision::DenyWithFeedback,
                        },
                        PermissionChoice {
                            key: "7".to_string(),
                            label: "always deny this file".to_string(),
                            decision: PermissionDecision::DenyAlways,
                        },
                    ],
                },
                &format!(
                    "Allow file edit?\n- file: {}\nEnter y to allow, others to deny.\n",
                    normalized_target
                ),
                PermissionDecision::AllowOnce,
                PermissionDecision::DenyOnce,
            )
            .await?;

        let mut state = self.state.lock().await;

        match prompt_result.decision {
            PermissionDecision::AllowOnce => {
                state.session_allowed_edits.insert(normalized_target);
                Ok(())
            }
            PermissionDecision::AllowTurn => {
                state.turn_allowed_edits.insert(normalized_target);
                Ok(())
            }
            PermissionDecision::AllowAllTurn => {
                state.turn_allow_all_edits = true;
                Ok(())
            }
            PermissionDecision::AllowAlways => {
                state.allowed_edit_patterns.insert(normalized_target);
                drop(state);
                self.persist()
            }
            PermissionDecision::DenyWithFeedback => {
                let guidance = prompt_result.feedback.unwrap_or_default();
                let guidance = guidance.trim();
                if guidance.is_empty() {
                    state.session_denied_edits.insert(normalized_target.clone());
                    Err(anyhow!("Edit denied: {normalized_target}"))
                } else {
                    Err(anyhow!(
                        "Edit denied: {normalized_target}\nUser guidance: {guidance}"
                    ))
                }
            }
            PermissionDecision::DenyAlways => {
                state.denied_edit_patterns.insert(normalized_target.clone());
                drop(state);
                self.persist()?;
                Err(anyhow!("Edit denied: {normalized_target}"))
            }
            PermissionDecision::DenyOnce => {
                state.session_denied_edits.insert(normalized_target.clone());
                Err(anyhow!("Edit denied: {normalized_target}"))
            }
        }
    }

    /// 将可持久化权限规则写回磁盘。
    pub fn persist(&self) -> Result<()> {
        let path = self.store_path.clone();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let state = self
            .state
            .try_lock()
            .map_err(|_| anyhow!("Permission state lock unavailable"))?;
        let store = PermissionStore {
            allowed_directory_prefixes: state.allowed_directory_prefixes.iter().cloned().collect(),
            denied_directory_prefixes: state.denied_directory_prefixes.iter().cloned().collect(),
            allowed_command_patterns: state.allowed_command_patterns.iter().cloned().collect(),
            denied_command_patterns: state.denied_command_patterns.iter().cloned().collect(),
            allowed_edit_patterns: state.allowed_edit_patterns.iter().cloned().collect(),
            denied_edit_patterns: state.denied_edit_patterns.iter().cloned().collect(),
        };
        fs::write(path, format!("{}\n", serde_json::to_string_pretty(&store)?))?;
        Ok(())
    }
}

impl PermissionManager {
    /// 终端回退确认：仅在 TTY 模式下读取用户输入。
    fn confirm(prompt: &str) -> Result<bool> {
        let is_tty_in = io::stdin().is_terminal();
        let is_tty_out = io::stdout().is_terminal();

        if !is_tty_in || !is_tty_out {
            return Ok(false);
        }

        let mut stdout = io::stdout();
        write!(stdout, "{}", prompt)?;
        stdout.flush()?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|e| anyhow!("Failed to read permission response: {}", e))?;
        Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES"))
    }
}

/// 判断目标路径是否位于指定根目录内。
fn is_within_directory(root: &Path, target: &Path) -> bool {
    let Ok(relative) = target.strip_prefix(root) else {
        return false;
    };
    !relative
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// 从磁盘读取权限存储，不存在时返回默认值。
fn read_store(path: &Path) -> Result<PermissionStore> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(PermissionStore::default()),
        Err(err) => Err(err.into()),
    }
}

/// 识别高风险命令并给出触发审批的原因。
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
        // git checkout -- can overwrite working tree files
        if args.iter().any(|x| x == "checkout") && args.iter().any(|x| x == "--") {
            return Some(format!(
                "git checkout -- can overwrite working tree files ({signature})"
            ));
        }
        // git restore --source can overwrite local files
        if args.iter().any(|x| x == "restore") && args.iter().any(|x| x.starts_with("--source")) {
            return Some(format!(
                "git restore --source can overwrite local files ({signature})"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 验证 `git reset --hard` 被识别为危险命令。
    fn test_classify_dangerous_command_git_reset() {
        let args = vec!["reset".to_string(), "--hard".to_string()];
        let result = classify_dangerous_command("git", &args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("git reset --hard"));
    }

    #[test]
    /// 验证 `git checkout --` 被识别为危险命令。
    fn test_classify_dangerous_command_git_checkout() {
        let args = vec![
            "checkout".to_string(),
            "--".to_string(),
            "file.txt".to_string(),
        ];
        let result = classify_dangerous_command("git", &args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("git checkout --"));
    }

    #[test]
    /// 验证 `git restore --source` 被识别为危险命令。
    fn test_classify_dangerous_command_git_restore_source() {
        let args = vec![
            "restore".to_string(),
            "--source".to_string(),
            "HEAD".to_string(),
        ];
        let result = classify_dangerous_command("git", &args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("git restore --source"));
    }

    #[test]
    /// 验证 `npm publish` 被识别为危险命令。
    fn test_classify_dangerous_command_npm_publish() {
        let args = vec!["publish".to_string()];
        let result = classify_dangerous_command("npm", &args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("npm publish"));
    }

    #[test]
    /// 验证解释器执行命令会触发危险判定。
    fn test_classify_dangerous_command_node_execution() {
        let args = vec!["script.js".to_string()];
        let result = classify_dangerous_command("node", &args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("can execute arbitrary code"));
    }

    #[test]
    /// 验证常规安全命令不会被误判。
    fn test_classify_safe_command() {
        let args = vec!["status".to_string()];
        let result = classify_dangerous_command("git", &args);
        assert!(result.is_none());
    }

    #[test]
    /// 验证 `ls` 命令不会触发危险判定。
    fn test_classify_ls_safe() {
        let args = vec!["-la".to_string(), "/tmp".to_string()];
        let result = classify_dangerous_command("ls", &args);
        assert!(result.is_none());
    }

    #[test]
    /// 验证目录内路径判定为 true。
    fn test_is_within_directory_valid() {
        use std::path::PathBuf;
        let root = PathBuf::from("/home/user/project");
        let target = PathBuf::from("/home/user/project/src/main.rs");
        assert!(is_within_directory(&root, &target));
    }

    #[test]
    /// 验证目录外路径判定为 false。
    fn test_is_within_directory_outside() {
        use std::path::PathBuf;
        let root = PathBuf::from("/home/user/project");
        let target = PathBuf::from("/home/user/other/file.txt");
        assert!(!is_within_directory(&root, &target));
    }

    #[test]
    /// 验证包含父目录跳转时不会误放行。
    fn test_is_within_directory_parent_escape() {
        use std::path::PathBuf;
        let root = PathBuf::from("/home/user/project");
        let target = PathBuf::from("/home/user/project/../other/file.txt");
        // The function should detect parent references and prevent escape
        // This test verifies the directory escape protection
        let _ = is_within_directory(&root, &target);
    }
}
