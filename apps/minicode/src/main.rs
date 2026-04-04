pub use minicode_agent_core::*;
pub use minicode_install::*;
pub use minicode_manage::*;
pub use minicode_mock_model::*;
pub use minicode_permissions::*;
pub use minicode_tools_runtime::*;
pub use minicode_tui::*;

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use minicode_core::config::load_runtime_config;
use minicode_core::types::ModelAdapter;

#[derive(Debug, Parser)]
#[command(
    name = "minicode",
    version,
    about = "MiniCode 命令行",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Install {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        _args: Vec<String>,
    },
    Mcp {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Skills {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Help,
}

fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn command_to_management_argv(command: &Commands) -> Option<Vec<String>> {
    match command {
        Commands::Mcp { args } => {
            let mut argv = vec!["mcp".to_string()];
            argv.extend(args.iter().cloned());
            Some(argv)
        }
        Commands::Skills { args } => {
            let mut argv = vec!["skills".to_string()];
            argv.extend(args.iter().cloned());
            Some(argv)
        }
        Commands::Help => Some(vec!["help".to_string()]),
        Commands::Install { .. } => None,
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        let _ = err;
        std::process::exit(1);
    }
}

async fn real_main() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cli = Cli::parse();

    if matches!(cli.command, Some(Commands::Install { .. })) {
        run_install_wizard(&cwd)?;
        return Ok(());
    }

    if let Some(command) = &cli.command
        && let Some(management_argv) = command_to_management_argv(command)
        && maybe_handle_management_command(&cwd, &management_argv).await?
    {
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

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let interactive = is_interactive_terminal();

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
