use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use minicode_config::{project_session_permissions_path, runtime_store};

use crate::{
    EnsureCommandOptions, PermissionChoice, PermissionDecision, PermissionManager,
    PermissionPromptHandler, PermissionPromptKind, PermissionPromptRequest, PermissionPromptResult,
    PermissionState, PermissionStore, classify_dangerous_command, is_within_directory, read_store,
};

impl PermissionManager {
    /// 从持久化存储加载权限配置并初始化管理器。
    pub fn new(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let cwd = runtime_store().cwd.clone();
        let session_id = runtime_store().session_id.clone();
        let store_path = project_session_permissions_path(&cwd, &session_id);
        let store = read_store(&store_path)?;

        let state = PermissionState {
            allowed_directory_prefixes: store.allowed_directory_prefixes.into_iter().collect(),
            denied_directory_prefixes: store.denied_directory_prefixes.into_iter().collect(),
            session_allowed_paths: std::collections::HashSet::new(),
            session_denied_paths: std::collections::HashSet::new(),
            allowed_command_patterns: store.allowed_command_patterns.into_iter().collect(),
            denied_command_patterns: store.denied_command_patterns.into_iter().collect(),
            session_allowed_commands: std::collections::HashSet::new(),
            session_denied_commands: std::collections::HashSet::new(),
            allowed_edit_patterns: store.allowed_edit_patterns.into_iter().collect(),
            denied_edit_patterns: store.denied_edit_patterns.into_iter().collect(),
            session_allowed_edits: std::collections::HashSet::new(),
            session_denied_edits: std::collections::HashSet::new(),
            turn_allowed_edits: std::collections::HashSet::new(),
            turn_allow_all_edits: false,
        };
        Ok(Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
            store_path,
            state: std::sync::Arc::new(tokio::sync::Mutex::new(state)),
            prompt_handler: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
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

    /// 校验路径访问权限，必要时触发审批。
    pub async fn ensure_path_access(&self, target_path: &str, intent: &str) -> Result<()> {
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

        let scope_directory = if matches!(intent, "list" | "command_cwd") {
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
