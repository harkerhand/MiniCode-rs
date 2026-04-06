use minicode_config::runtime_store;
use minicode_types::PermissionSummaryItem;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct PermissionStore {
    #[serde(default)]
    pub(crate) allowed_directory_prefixes: Vec<String>,
    #[serde(default)]
    pub(crate) denied_directory_prefixes: Vec<String>,
    #[serde(default)]
    pub(crate) allowed_command_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) denied_command_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) allowed_edit_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) denied_edit_patterns: Vec<String>,
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

pub(crate) type PermissionPromptFuture =
    Pin<Box<dyn Future<Output = PermissionPromptResult> + Send>>;
pub type PermissionPromptHandler =
    Arc<dyn Fn(PermissionPromptRequest) -> PermissionPromptFuture + Send + Sync>;

#[derive(Debug, Clone, Default)]
pub struct EnsureCommandOptions {
    pub force_prompt_reason: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PermissionState {
    pub(crate) allowed_directory_prefixes: HashSet<String>,
    pub(crate) denied_directory_prefixes: HashSet<String>,
    pub(crate) session_allowed_paths: HashSet<String>,
    pub(crate) session_denied_paths: HashSet<String>,
    pub(crate) allowed_command_patterns: HashSet<String>,
    pub(crate) denied_command_patterns: HashSet<String>,
    pub(crate) session_allowed_commands: HashSet<String>,
    pub(crate) session_denied_commands: HashSet<String>,
    pub(crate) allowed_edit_patterns: HashSet<String>,
    pub(crate) denied_edit_patterns: HashSet<String>,
    pub(crate) session_allowed_edits: HashSet<String>,
    pub(crate) session_denied_edits: HashSet<String>,
    pub(crate) turn_allowed_edits: HashSet<String>,
    pub(crate) turn_allow_all_edits: bool,
}

#[derive(Clone)]
pub struct PermissionManager {
    pub(crate) state: Arc<Mutex<PermissionState>>,
    pub(crate) prompt_handler: Arc<Mutex<Option<PermissionPromptHandler>>>,
}

impl PermissionManager {
    /// 返回权限状态的简要摘要文本。
    pub fn get_summary(&self) -> Vec<PermissionSummaryItem> {
        let mut output = Vec::new();
        let state = self.state.try_lock().ok();
        let cwd = runtime_store().cwd.to_string_lossy().to_string();
        output.push(PermissionSummaryItem::Cwd(cwd));
        let empty_dirs = state
            .as_ref()
            .map(|x| x.allowed_directory_prefixes.is_empty())
            .unwrap_or(true);
        if empty_dirs {
            output.push(PermissionSummaryItem::ExtraAllowDirs(Vec::new()));
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
            output.push(PermissionSummaryItem::ExtraAllowDirs(dirs));
        }
        let empty_cmds = state
            .as_ref()
            .map(|x| x.allowed_command_patterns.is_empty())
            .unwrap_or(true);
        if empty_cmds {
            output.push(PermissionSummaryItem::DangerousAllowDirs(Vec::new()));
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
            output.push(PermissionSummaryItem::DangerousAllowDirs(cmds));
        }
        output
    }

    pub fn get_summary_text(&self) -> Vec<String> {
        self.get_summary()
            .into_iter()
            .map(|item| item.to_string())
            .collect()
    }
}
