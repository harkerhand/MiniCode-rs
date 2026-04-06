use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use minicode_config::{config_from_file, mini_code_settings_path, save_minicode_settings};

/// 读取一行用户输入，支持默认值回填。
fn prompt_line(prompt: &str, default: Option<&str>) -> Result<String> {
    let mut stdout = io::stdout();
    if let Some(d) = default {
        write!(stdout, "{} [{}]: ", prompt, d)?;
    } else {
        write!(stdout, "{}: ", prompt)?;
    }
    stdout.flush()?;

    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let value = buf.trim().to_string();
    if !value.is_empty() {
        Ok(value)
    } else if let Some(d) = default {
        Ok(d.to_string())
    } else {
        Err(anyhow::anyhow!("Input cannot be empty"))
    }
}

/// 检查 PATH 中是否已包含目标目录。
fn has_path_entry(target: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|p| p == target)
}

/// 拷贝当前可执行文件到目标路径，并设置可执行权限。
fn copy_launcher_exe(launcher_path: impl AsRef<Path>, binary_path: impl AsRef<Path>) -> Result<()> {
    std::fs::copy(&binary_path, &launcher_path)?;
    let mut perms = std::fs::metadata(&launcher_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&launcher_path, perms)?;
    Ok(())
}

/// 交互式安装向导：收集配置并写入启动脚本。
pub fn run_install_wizard(cwd: impl AsRef<Path>) -> Result<()> {
    println!("mini-code installer");

    let settings_path = mini_code_settings_path();
    println!(
        "Configuration will be written to: {}",
        settings_path.display()
    );
    println!("Settings are stored separately and won't affect other local tool configurations.");
    println!();

    let mut effective = config_from_file(cwd)?;

    if effective.model.is_empty() {
        let env_model = std::env::var("ANTHROPIC_MODEL").ok();
        if let Some(env_model) = env_model {
            effective.model = env_model;
        }
    }
    if effective.base_url.is_empty() {
        let env_base_url = std::env::var("ANTHROPIC_BASE_URL").ok();
        if let Some(env_base_url) = env_base_url {
            effective.base_url = env_base_url;
        }
    }
    if let Some(auth_token_default) = &effective.auth_token
        && auth_token_default.is_empty()
    {
        effective.auth_token = std::env::var("ANTHROPIC_AUTH_TOKEN").ok();
    }

    let model = prompt_line("Model name", Some(effective.model.as_str()))?;
    effective.model = model;
    let base_url = prompt_line("ANTHROPIC_BASE_URL", Some(effective.base_url.as_str()))?;
    effective.base_url = base_url;

    let saved_token_suffix = if effective.auth_token.is_some() {
        " [saved]"
    } else {
        " [not set]"
    };
    let mut stdout = io::stdout();
    write!(stdout, "ANTHROPIC_AUTH_TOKEN{}: ", saved_token_suffix)?;
    stdout.flush()?;
    let mut token_input = String::new();
    io::stdin().read_line(&mut token_input)?;
    let auth_token = token_input.trim();
    let auth_token = if !auth_token.is_empty() {
        auth_token.to_string()
    } else if let Some(saved) = &effective.auth_token {
        saved.clone()
    } else {
        return Err(anyhow::anyhow!("ANTHROPIC_AUTH_TOKEN cannot be empty"));
    };
    effective.auth_token = Some(auth_token);

    save_minicode_settings(&effective)?;

    let target_bin = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".local")
        .join("bin");
    std::fs::create_dir_all(&target_bin)?;

    let launcher_path = target_bin.join("minicode");
    let binary_path = std::env::current_exe()?;
    copy_launcher_exe(&launcher_path, &binary_path)?;

    println!();
    println!("Installation complete.");
    println!("Configuration file: {}", settings_path.display());
    println!("Launcher command: {}", launcher_path.display());

    if !has_path_entry(target_bin.to_string_lossy().as_ref()) {
        println!();
        println!("Note: {} is not in your PATH.", target_bin.display());
        println!("You can add it to ~/.bashrc or ~/.zshrc:");
        println!("export PATH=\"{}:$PATH\"", target_bin.display());
    } else {
        println!();
        println!("You can now run `minicode` from any terminal.");
    }

    Ok(())
}
