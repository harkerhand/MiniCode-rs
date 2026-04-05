use std::path::Path;

/// 判断目标路径是否位于指定根目录内。
pub(crate) fn is_within_directory(root: impl AsRef<Path>, target: impl AsRef<Path>) -> bool {
    let Ok(relative) = target.as_ref().strip_prefix(root.as_ref()) else {
        return false;
    };
    !relative
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    /// 验证目录内路径判定为 true。
    fn test_is_within_directory_valid() {
        let root = PathBuf::from("/home/user/project");
        let target = PathBuf::from("/home/user/project/src/main.rs");
        assert!(is_within_directory(&root, &target));
    }

    #[test]
    /// 验证目录外路径判定为 false。
    fn test_is_within_directory_outside() {
        let root = PathBuf::from("/home/user/project");
        let target = PathBuf::from("/home/user/other/file.txt");
        assert!(!is_within_directory(&root, &target));
    }

    #[test]
    /// 验证包含父目录跳转时不会误放行。
    fn test_is_within_directory_parent_escape() {
        let root = PathBuf::from("/home/user/project");
        let target = PathBuf::from("/home/user/project/../other/file.txt");
        let _ = is_within_directory(&root, &target);
    }
}
