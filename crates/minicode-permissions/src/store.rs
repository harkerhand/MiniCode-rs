use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::PermissionStore;

/// 从磁盘读取权限存储，不存在时返回默认值。
pub(crate) fn read_store(path: impl AsRef<Path>) -> Result<PermissionStore> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(PermissionStore::default()),
        Err(err) => Err(err.into()),
    }
}
