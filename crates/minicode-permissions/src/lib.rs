mod manager;
mod path_utils;
mod rules;
mod store;
mod types;

use std::sync::LazyLock;

pub(crate) use path_utils::is_within_directory;
pub(crate) use rules::classify_dangerous_command;
pub(crate) use store::read_store;
pub use types::*;

static PERMISSIONS: LazyLock<PermissionManager> = LazyLock::new(PermissionManager::default);

pub fn get_permission_manager() -> &'static PermissionManager {
    &PERMISSIONS
}
