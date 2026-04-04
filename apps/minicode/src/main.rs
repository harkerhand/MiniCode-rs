mod agent_loop;
mod anthropic_adapter;
mod background_tasks;
mod cli_commands;
mod config;
mod file_review;
mod history;
mod install;
mod local_tool_shortcuts;
mod manage_cli;
mod mcp;
mod mock_model;
mod permissions;
mod prompt;
mod skills;
mod tool;
mod tools;
mod tui;
mod types;
mod workspace;

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use anthropic_adapter::AnthropicModelAdapter;
use config::load_runtime_config;
use manage_cli::maybe_handle_management_command;
use mock_model::MockModelAdapter;
use permissions::PermissionManager;
use tools::create_default_tool_registry;
use tui::{TuiAppArgs, run_tui_app};
use types::ModelAdapter;

fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn should_force_tui(argv: &[String]) -> bool {
    argv.iter().any(|x| x == "--tui")
        || std::env::var("MINI_CODE_FORCE_TUI").ok().as_deref() == Some("1")
}

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}

async fn real_main() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let argv = std::env::args().skip(1).collect::<Vec<_>>();

    if argv.first().map(|x| x.as_str()) == Some("install") {
        install::run_install_wizard(&cwd)?;
        return Ok(());
    }

    if maybe_handle_management_command(&cwd, &argv).await? {
        return Ok(());
    }

    let runtime = load_runtime_config(&cwd).ok();
    let tools = Arc::new(create_default_tool_registry(&cwd, runtime.as_ref()).await?);
    let permissions = PermissionManager::new(cwd.clone())?;

    let model: Arc<dyn ModelAdapter> =
        if std::env::var("MINI_CODE_MODEL_MODE").ok().as_deref() == Some("mock") {
            Arc::new(MockModelAdapter)
        } else {
            Arc::new(AnthropicModelAdapter::new(tools.clone(), cwd.clone()))
        };

    let force_tui = should_force_tui(&argv);

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let interactive = if force_tui {
        if !(stdin_tty && stdout_tty) {
            return Err(anyhow!(
                "--tui 已指定，但当前终端不支持 TUI（stdin_tty={}, stdout_tty={}）。",
                stdin_tty,
                stdout_tty
            ));
        }
        true
    } else {
        is_interactive_terminal()
    };

    if !interactive {
        return Err(anyhow!(
            "当前仅支持 ratatui 交互模式：需要在 TTY 终端中运行（stdin_tty={}, stdout_tty={}）。",
            stdin_tty,
            stdout_tty
        ));
    }

    run_tui_app(TuiAppArgs {
        runtime,
        tools: tools.clone(),
        model,
        cwd,
        permissions,
    })
    .await?;

    tools.dispose().await;
    Ok(())
}
