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

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use agent_loop::run_agent_turn;
use anthropic_adapter::AnthropicModelAdapter;
use cli_commands::{find_matching_slash_commands, try_handle_local_command};
use config::load_runtime_config;
use history::{load_history_entries, save_history_entries};
use local_tool_shortcuts::parse_local_tool_shortcut;
use manage_cli::maybe_handle_management_command;
use mock_model::MockModelAdapter;
use permissions::PermissionManager;
use prompt::build_system_prompt;
use tool::ToolContext;
use tools::create_default_tool_registry;
use tui::{TuiAppArgs, run_tui_app};
use types::{ChatMessage, ModelAdapter};

fn render_banner(
    runtime: Option<&config::RuntimeConfig>,
    cwd: &str,
    permission_summary: &[String],
    stats: (usize, usize, usize, usize),
) -> String {
    let (transcript_count, message_count, skill_count, mcp_count) = stats;
    let model = runtime
        .map(|x| x.model.clone())
        .unwrap_or_else(|| "(unconfigured)".to_string());
    format!(
        "MiniCode-RS | model={} | cwd={}\n{}\ntranscript={} messages={} skills={} mcp={}",
        model,
        cwd,
        permission_summary.join(" | "),
        transcript_count,
        message_count,
        skill_count,
        mcp_count
    )
}

fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn should_force_tui(argv: &[String]) -> bool {
    argv.iter().any(|x| x == "--tui")
        || std::env::var("MINI_CODE_FORCE_TUI").ok().as_deref() == Some("1")
}

fn should_force_repl(argv: &[String]) -> bool {
    argv.iter().any(|x| x == "--repl" || x == "--no-tui")
        || std::env::var("MINI_CODE_NO_TUI").ok().as_deref() == Some("1")
}

async fn run_repl_loop(
    cwd: PathBuf,
    runtime: Option<config::RuntimeConfig>,
    tools: Arc<tool::ToolRegistry>,
    mut permissions: PermissionManager,
    model: Arc<dyn ModelAdapter>,
    interactive: bool,
) -> Result<()> {
    let mut history = load_history_entries();
    let mut messages = vec![ChatMessage::System {
        content: build_system_prompt(
            &cwd,
            &permissions.get_summary(),
            tools.get_skills(),
            tools.get_mcp_servers(),
        ),
    }];

    println!(
        "{}\n",
        render_banner(
            runtime.as_ref(),
            &cwd.to_string_lossy(),
            &permissions.get_summary(),
            (
                0,
                messages.len(),
                tools.get_skills().len(),
                tools.get_mcp_servers().len()
            )
        )
    );

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    if interactive {
        print!("minicode> ");
        stdout.flush()?;
    }

    for line in stdin.lock().lines() {
        let raw_input = line?;
        let input = raw_input.trim().to_string();
        if input.is_empty() {
            if interactive {
                print!("minicode> ");
                stdout.flush()?;
            }
            continue;
        }
        if input == "/exit" {
            break;
        }

        history.push(input.clone());

        if input == "/tools" {
            println!(
                "\n{}\n",
                tools
                    .list()
                    .iter()
                    .map(|t| format!("{}: {}", t.name(), t.description()))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            if interactive {
                print!("minicode> ");
                stdout.flush()?;
            }
            continue;
        }

        if input.starts_with("/ls")
            || input.starts_with("/grep")
            || input.starts_with("/read")
            || input.starts_with("/write")
            || input.starts_with("/modify")
            || input.starts_with("/edit")
            || input.starts_with("/patch")
            || input.starts_with("/cmd")
        {
            if let Some(shortcut) = parse_local_tool_shortcut(&input) {
                let result = tools
                    .execute(
                        shortcut.tool_name,
                        shortcut.input,
                        &ToolContext {
                            cwd: cwd.to_string_lossy().to_string(),
                            permissions: Some(Arc::new(permissions.clone())),
                        },
                    )
                    .await;
                println!("\n{}\n", result.output);
                if interactive {
                    print!("minicode> ");
                    stdout.flush()?;
                }
                continue;
            }
        }

        if input.starts_with('/') {
            if let Some(output) = try_handle_local_command(&input, &cwd, Some(&tools)).await? {
                println!("\n{}\n", output);
                if interactive {
                    print!("minicode> ");
                    stdout.flush()?;
                }
                continue;
            }

            let matches = find_matching_slash_commands(&input);
            if !matches.is_empty() {
                println!("\n未识别命令。你是不是想输入：\n{}\n", matches.join("\n"));
            } else {
                println!("\n未识别命令。输入 /help 查看可用命令。\n");
            }
            if interactive {
                print!("minicode> ");
                stdout.flush()?;
            }
            continue;
        }

        messages[0] = ChatMessage::System {
            content: build_system_prompt(
                &cwd,
                &permissions.get_summary(),
                tools.get_skills(),
                tools.get_mcp_servers(),
            ),
        };
        messages.push(ChatMessage::User {
            content: input.clone(),
        });

        permissions.begin_turn();
        let updated = run_agent_turn(
            model.as_ref(),
            &tools,
            messages,
            ToolContext {
                cwd: cwd.to_string_lossy().to_string(),
                permissions: Some(Arc::new(permissions.clone())),
            },
            None,
            None,
        )
        .await;

        permissions.end_turn();
        let assistant = updated.iter().rev().find_map(|m| {
            if let ChatMessage::Assistant { content } = m {
                Some(content.clone())
            } else {
                None
            }
        });
        if let Some(reply) = assistant {
            println!("\n{}\n", reply);
        }

        messages = updated;

        if interactive {
            print!("minicode> ");
            stdout.flush()?;
        }
    }

    let _ = save_history_entries(&history);

    Ok(())
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
        install::run_install_wizard()?;
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
    let force_repl = should_force_repl(&argv);
    if force_tui && force_repl {
        return Err(anyhow!("参数冲突：不能同时使用 --tui 和 --repl/--no-tui。"));
    }

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let interactive = if force_repl {
        false
    } else if force_tui {
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

    if !interactive && stdout_tty {
        eprintln!(
            "未进入 TUI：stdin_tty={}, stdout_tty={}。可尝试在真实终端直接运行，或使用 --tui 强制检测。",
            stdin_tty, stdout_tty
        );
    }

    if interactive {
        run_tui_app(TuiAppArgs {
            runtime,
            tools: tools.clone(),
            model,
            cwd,
            permissions,
        })
        .await?;
    } else {
        run_repl_loop(cwd, runtime, tools.clone(), permissions, model, false).await?;
    }
    tools.dispose().await;
    Ok(())
}
