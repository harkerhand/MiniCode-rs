use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};

use crate::config::{
    McpServerConfig, load_scoped_mcp_servers, mini_code_mcp_path, project_mcp_path,
    save_scoped_mcp_servers,
};
use crate::skills::{discover_skills, install_skill, remove_managed_skill};

fn print_usage() {
    println!(
        "minicode management commands\n\nminicode mcp list [--project]\nminicode mcp add <name> [--project] [--protocol <auto|content-length|newline-json>] [--env KEY=VALUE ...] -- <command> [args...]\nminicode mcp remove <name> [--project]\n\nminicode skills list\nminicode skills add <path-to-skill-or-dir> [--name <name>] [--project]\nminicode skills remove <name> [--project]"
    );
}

fn parse_scope(args: &[String]) -> (String, Vec<String>) {
    let mut rest = args.to_vec();
    if let Some(idx) = rest.iter().position(|x| x == "--project") {
        rest.remove(idx);
        return ("project".to_string(), rest);
    }
    ("user".to_string(), rest)
}

fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    if let Some(idx) = args.iter().position(|x| x == name) {
        if idx + 1 >= args.len() {
            return Err(anyhow!("Missing value for {name}"));
        }
        let value = args[idx + 1].clone();
        args.remove(idx + 1);
        args.remove(idx);
        return Ok(Some(value));
    }
    Ok(None)
}

fn take_repeat_option(args: &mut Vec<String>, name: &str) -> Result<Vec<String>> {
    let mut values = vec![];
    while let Some(idx) = args.iter().position(|x| x == name) {
        if idx + 1 >= args.len() {
            return Err(anyhow!("Missing value for {name}"));
        }
        let value = args[idx + 1].clone();
        args.remove(idx + 1);
        args.remove(idx);
        values.push(value);
    }
    Ok(values)
}

fn parse_env_pairs(entries: &[String]) -> Result<HashMap<String, serde_json::Value>> {
    let mut env = HashMap::new();
    for entry in entries {
        let Some(idx) = entry.find('=') else {
            return Err(anyhow!("Invalid --env value: {entry}"));
        };
        let k = entry[..idx].trim();
        let v = entry[idx + 1..].to_string();
        if k.is_empty() {
            return Err(anyhow!("Invalid --env value: {entry}"));
        }
        env.insert(k.to_string(), serde_json::Value::String(v));
    }
    Ok(env)
}

async fn handle_mcp_command(cwd: &Path, args: &[String]) -> Result<bool> {
    let Some(subcommand) = args.first() else {
        print_usage();
        return Ok(true);
    };

    let (scope, rest) = parse_scope(&args[1..]);

    match subcommand.as_str() {
        "list" => {
            let servers = load_scoped_mcp_servers(&scope, cwd)?;
            if servers.is_empty() {
                let path = if scope == "project" {
                    project_mcp_path(cwd)
                } else {
                    mini_code_mcp_path()
                };
                println!("No MCP servers configured in {}.", path.display());
                return Ok(true);
            }
            for (name, server) in servers {
                let args = server.args.unwrap_or_default().join(" ");
                let protocol = server
                    .protocol
                    .map(|x| format!(" protocol={x}"))
                    .unwrap_or_default();
                println!("{}: {} {}{}", name, server.command, args, protocol);
            }
            Ok(true)
        }
        "add" => {
            let sep = rest
                .iter()
                .position(|x| x == "--")
                .ok_or_else(|| anyhow!("Use -- before MCP command."))?;
            let mut head = rest[..sep].to_vec();
            let command_parts = rest[sep + 1..].to_vec();
            if command_parts.is_empty() {
                return Err(anyhow!("Missing MCP command after --."));
            }
            if head.is_empty() {
                return Err(anyhow!("Missing MCP server name."));
            }
            let name = head.remove(0);
            let protocol = take_option(&mut head, "--protocol")?;
            let env = parse_env_pairs(&take_repeat_option(&mut head, "--env")?)?;
            if !head.is_empty() {
                return Err(anyhow!("Unknown arguments: {}", head.join(" ")));
            }

            let mut existing = load_scoped_mcp_servers(&scope, cwd)?;
            existing.insert(
                name.clone(),
                McpServerConfig {
                    command: command_parts[0].clone(),
                    args: if command_parts.len() > 1 {
                        Some(command_parts[1..].to_vec())
                    } else {
                        Some(vec![])
                    },
                    env: if env.is_empty() { None } else { Some(env) },
                    cwd: None,
                    enabled: None,
                    protocol,
                },
            );
            save_scoped_mcp_servers(&scope, cwd, existing)?;
            println!("Added MCP server {}", name);
            Ok(true)
        }
        "remove" => {
            let Some(name) = rest.first() else {
                return Err(anyhow!("Missing MCP server name."));
            };
            let mut existing = load_scoped_mcp_servers(&scope, cwd)?;
            if existing.remove(name).is_none() {
                println!("MCP server {} not found", name);
                return Ok(true);
            }
            save_scoped_mcp_servers(&scope, cwd, existing)?;
            println!("Removed MCP server {}", name);
            Ok(true)
        }
        _ => {
            print_usage();
            Ok(true)
        }
    }
}

async fn handle_skills_command(cwd: &Path, args: &[String]) -> Result<bool> {
    let Some(subcommand) = args.first() else {
        print_usage();
        return Ok(true);
    };
    let (scope, mut rest) = parse_scope(&args[1..]);

    match subcommand.as_str() {
        "list" => {
            let skills = discover_skills(cwd);
            if skills.is_empty() {
                println!("No skills discovered.");
                return Ok(true);
            }
            for skill in skills {
                println!("{}: {} ({})", skill.name, skill.description, skill.path);
            }
            Ok(true)
        }
        "add" => {
            let Some(source_path) = rest.first().cloned() else {
                return Err(anyhow!("Missing skill source path."));
            };
            rest.remove(0);
            let name = take_option(&mut rest, "--name")?;
            let (name, target) = install_skill(cwd, &source_path, name, &scope)?;
            println!("Installed skill {} at {}", name, target);
            Ok(true)
        }
        "remove" => {
            let Some(name) = rest.first().cloned() else {
                return Err(anyhow!("Missing skill name."));
            };
            let (removed, target) = remove_managed_skill(cwd, &name, &scope)?;
            if !removed {
                println!("Skill {} not found at {}", name, target);
                return Ok(true);
            }
            println!("Removed skill {} from {}", name, target);
            Ok(true)
        }
        _ => {
            print_usage();
            Ok(true)
        }
    }
}

pub async fn maybe_handle_management_command(cwd: &Path, argv: &[String]) -> Result<bool> {
    let Some(category) = argv.first() else {
        return Ok(false);
    };

    match category.as_str() {
        "mcp" => handle_mcp_command(cwd, &argv[1..]).await,
        "skills" => handle_skills_command(cwd, &argv[1..]).await,
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(true)
        }
        _ => Ok(false),
    }
}
