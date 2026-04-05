/// 识别高风险命令并给出触发审批的原因。
pub(crate) fn classify_dangerous_command(command: &str, args: &[String]) -> Option<String> {
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
        if args.iter().any(|x| x == "checkout") && args.iter().any(|x| x == "--") {
            return Some(format!(
                "git checkout -- can overwrite working tree files ({signature})"
            ));
        }
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
        assert!(result.expect("reason").contains("git reset --hard"));
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
        assert!(result.expect("reason").contains("git checkout --"));
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
        assert!(result.expect("reason").contains("git restore --source"));
    }

    #[test]
    /// 验证 `npm publish` 被识别为危险命令。
    fn test_classify_dangerous_command_npm_publish() {
        let args = vec!["publish".to_string()];
        let result = classify_dangerous_command("npm", &args);
        assert!(result.is_some());
        assert!(result.expect("reason").contains("npm publish"));
    }

    #[test]
    /// 验证解释器执行命令会触发危险判定。
    fn test_classify_dangerous_command_node_execution() {
        let args = vec!["script.js".to_string()];
        let result = classify_dangerous_command("node", &args);
        assert!(result.is_some());
        assert!(
            result
                .expect("reason")
                .contains("can execute arbitrary code")
        );
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
}
