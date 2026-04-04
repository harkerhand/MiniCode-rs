use std::io::{self, Write};

use anyhow::Result;

use crate::config::{MiniCodeSettings, save_minicode_settings};

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
    if value.is_empty() {
        Ok(default.unwrap_or("").to_string())
    } else {
        Ok(value)
    }
}

pub fn run_install_wizard() -> Result<()> {
    let model_default = std::env::var("ANTHROPIC_MODEL").ok();
    let base_default = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| Some("https://api.anthropic.com".to_string()));

    let model = prompt_line("输入模型名", model_default.as_deref())?;
    let base_url = prompt_line("输入 ANTHROPIC_BASE_URL", base_default.as_deref())?;
    let auth_token = prompt_line("输入 ANTHROPIC_AUTH_TOKEN", None)?;

    let mut env = std::collections::HashMap::new();
    if !base_url.trim().is_empty() {
        env.insert(
            "ANTHROPIC_BASE_URL".to_string(),
            serde_json::Value::String(base_url),
        );
    }
    if !auth_token.trim().is_empty() {
        env.insert(
            "ANTHROPIC_AUTH_TOKEN".to_string(),
            serde_json::Value::String(auth_token),
        );
    }

    save_minicode_settings(MiniCodeSettings {
        model: if model.trim().is_empty() {
            None
        } else {
            Some(model)
        },
        env: if env.is_empty() { None } else { Some(env) },
        ..MiniCodeSettings::default()
    })?;

    println!("已写入 ~/.mini-code/settings.json");
    Ok(())
}
