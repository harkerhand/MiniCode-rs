mod agent_loop;
mod anthropic_adapter;
mod cli_commands;
mod config;
mod file_review;
mod history;
mod install;
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

use anyhow::Result;

use agent_loop::run_agent_turn;
use anthropic_adapter::AnthropicModelAdapter;
use cli_commands::{find_matching_slash_commands, try_handle_local_command};
use config::load_runtime_config;
use history::{load_history_entries, save_history_entries};
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
            let (tool_name, payload) = parse_shortcut_command(&input);
            if let Some(tool_name) = tool_name {
                let result = tools
                    .execute(
                        tool_name,
                        payload,
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

fn parse_shortcut_command(input: &str) -> (Option<&'static str>, serde_json::Value) {
    if input == "/ls" {
        return (
            Some("list_files"),
            serde_json::json!({
                "path": ".",
            }),
        );
    }
    if let Some(path) = input.strip_prefix("/ls ") {
        return (
            Some("list_files"),
            serde_json::json!({ "path": path.trim() }),
        );
    }

    if let Some(rest) = input.strip_prefix("/grep ") {
        let parts = rest.split("::").collect::<Vec<_>>();
        if parts.len() == 2 {
            return (
                Some("grep_files"),
                serde_json::json!({ "pattern": parts[0].trim(), "path": parts[1].trim() }),
            );
        }
        return (
            Some("grep_files"),
            serde_json::json!({ "pattern": rest.trim() }),
        );
    }

    if let Some(path) = input.strip_prefix("/read ") {
        return (
            Some("read_file"),
            serde_json::json!({ "path": path.trim() }),
        );
    }

    if let Some(rest) = input.strip_prefix("/write ") {
        let parts = rest.splitn(2, "::").collect::<Vec<_>>();
        if parts.len() == 2 {
            return (
                Some("write_file"),
                serde_json::json!({ "path": parts[0].trim(), "content": parts[1] }),
            );
        }
    }

    if let Some(rest) = input.strip_prefix("/modify ") {
        let parts = rest.splitn(2, "::").collect::<Vec<_>>();
        if parts.len() == 2 {
            return (
                Some("modify_file"),
                serde_json::json!({ "path": parts[0].trim(), "content": parts[1] }),
            );
        }
    }

    if let Some(rest) = input.strip_prefix("/edit ") {
        let parts = rest.splitn(3, "::").collect::<Vec<_>>();
        if parts.len() == 3 {
            return (
                Some("edit_file"),
                serde_json::json!({
                    "path": parts[0].trim(),
                    "search": parts[1],
                    "replace": parts[2]
                }),
            );
        }
    }

    if let Some(rest) = input.strip_prefix("/patch ") {
        let parts = rest.split("::").collect::<Vec<_>>();
        if parts.len() >= 3 && parts.len() % 2 == 1 {
            let path = parts[0].trim();
            let mut replacements = vec![];
            let mut i = 1;
            while i + 1 < parts.len() {
                replacements.push(serde_json::json!({
                    "search": parts[i],
                    "replace": parts[i + 1],
                }));
                i += 2;
            }
            return (
                Some("patch_file"),
                serde_json::json!({ "path": path, "replacements": replacements }),
            );
        }
    }

    if let Some(rest) = input.strip_prefix("/cmd ") {
        if let Some((cwd, cmd)) = rest.split_once("::") {
            return (
                Some("run_command"),
                serde_json::json!({ "cwd": cwd.trim(), "command": cmd.trim() }),
            );
        }
        return (
            Some("run_command"),
            serde_json::json!({ "command": rest.trim() }),
        );
    }

    (None, serde_json::Value::Null)
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

    let interactive = is_interactive_terminal();
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
