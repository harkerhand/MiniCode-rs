use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use minicode_config::{
    MiniCodeSettings, load_effective_settings, mini_code_settings_path, save_minicode_settings,
};

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

/// 从 JSON 环境变量映射中读取非空字符串值。
fn get_env_string(
    env: &std::collections::HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    env.get(key)
        .map(|x| x.to_string().trim_matches('"').to_string())
        .filter(|x| !x.trim().is_empty())
}

/// 生成可执行启动脚本并赋予执行权限。
fn create_launcher_script(launcher_path: &Path, binary_path: &Path) -> Result<()> {
    let script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n\nexec \"{}\" \"$@\"\n",
        binary_path.display()
    );

    std::fs::write(launcher_path, script)?;
    let mut perms = std::fs::metadata(launcher_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(launcher_path, perms)?;
    Ok(())
}

/// 交互式安装向导：收集配置并写入启动脚本。
pub fn run_install_wizard(cwd: &Path) -> Result<()> {
    println!("mini-code installer");

    let settings_path = mini_code_settings_path();
    println!(
        "Configuration will be written to: {}",
        settings_path.display()
    );
    println!("Settings are stored separately and won't affect other local tool configurations.");
    println!();

    let effective = load_effective_settings(cwd)?;
    let effective_env = effective.env.unwrap_or_default();

    let model_default = effective
        .model
        .or_else(|| get_env_string(&effective_env, "ANTHROPIC_MODEL"))
        .or_else(|| std::env::var("ANTHROPIC_MODEL").ok());
    let base_url_default = get_env_string(&effective_env, "ANTHROPIC_BASE_URL")
        .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
        .or_else(|| Some("https://api.anthropic.com".to_string()));
    let auth_token_default = get_env_string(&effective_env, "ANTHROPIC_AUTH_TOKEN")
        .or_else(|| std::env::var("ANTHROPIC_AUTH_TOKEN").ok());

    let model = prompt_line("Model name", model_default.as_deref())?;
    let base_url = prompt_line("ANTHROPIC_BASE_URL", base_url_default.as_deref())?;

    let saved_token_suffix = if auth_token_default.is_some() {
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
    } else if let Some(saved) = &auth_token_default {
        saved.clone()
    } else {
        return Err(anyhow::anyhow!("ANTHROPIC_AUTH_TOKEN cannot be empty"));
    };

    let mut env = std::collections::HashMap::new();
    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        serde_json::Value::String(base_url.clone()),
    );
    env.insert(
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        serde_json::Value::String(auth_token.clone()),
    );
    env.insert(
        "ANTHROPIC_MODEL".to_string(),
        serde_json::Value::String(model.clone()),
    );

    save_minicode_settings(MiniCodeSettings {
        model: Some(model.clone()),
        env: Some(env),
        ..MiniCodeSettings::default()
    })?;

    let target_bin = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".local")
        .join("bin");
    std::fs::create_dir_all(&target_bin)?;

    let launcher_path = target_bin.join("minicode");
    let binary_path = std::env::current_exe()?;
    create_launcher_script(&launcher_path, &binary_path)?;

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
