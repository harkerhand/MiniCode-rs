mod manager;
mod path_utils;
mod rules;
mod store;
mod types;

use std::sync::OnceLock;

pub(crate) use path_utils::is_within_directory;
pub(crate) use rules::classify_dangerous_command;
pub(crate) use store::read_store;
pub use types::*;

static PERMISSIONS: OnceLock<PermissionManager> = OnceLock::new();

pub fn get_permission_manager() -> &'static PermissionManager {
    PERMISSIONS.get_or_init(PermissionManager::default)
}
