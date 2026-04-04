use minicode_permissions::session_permissions;
use minicode_types::PermissionSummaryItem;
use ratatui::text::{Line, Span};

use crate::state::{ScreenState, TuiAppArgs};
use crate::theme::theme;

pub(super) fn build_header_lines(args: &TuiAppArgs, state: &ScreenState) -> Vec<Line<'static>> {
    let theme = theme();
    let tools_count = args.tools.list().len();
    let skills_count = args.tools.get_skills().len();
    let mcp_count = args.tools.get_mcp_servers().len();
    let running_tasks = minicode_background_tasks::list_background_tasks()
        .into_iter()
        .filter(|task| task.status == "running")
        .count();
    let model = args
        .runtime
        .as_ref()
        .map(|x| x.model.clone())
        .unwrap_or_else(|| "(unconfigured)".to_string());
    let provider = args
        .runtime
        .as_ref()
        .map(|x| {
            x.base_url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("custom")
                .to_string()
        })
        .unwrap_or_else(|| "offline".to_string());
    let auth = args
        .runtime
        .as_ref()
        .map(|x| {
            if x.auth_token.is_some() {
                "auth_token"
            } else if x.api_key.is_some() {
                "api_key"
            } else {
                "none"
            }
        })
        .unwrap_or("none");
    let recent = state
        .recent_tools
        .iter()
        .rev()
        .take(3)
        .map(|(name, ok)| format!("{}:{}", name, if *ok { "ok" } else { "err" }))
        .collect::<Vec<_>>()
        .join(", ");

    vec![
        Line::from(vec![
            Span::styled("project", theme.header_label_project_style()),
            Span::raw(" "),
            Span::raw(args.cwd.display().to_string()),
            Span::raw("   "),
            Span::styled("provider", theme.header_label_provider_style()),
            Span::raw(" "),
            Span::raw(provider),
            Span::raw("   "),
            Span::styled("model", theme.header_label_model_style()),
            Span::raw(" "),
            Span::raw(model),
            Span::raw("   "),
            Span::styled("auth", theme.header_label_auth_style()),
            Span::raw(" "),
            Span::raw(auth),
        ]),
        Line::from(vec![
            Span::styled("session", theme.header_label_session_style()),
            Span::raw(format!(
                " messages={} events={} tools={} skills={} mcp={} running={}",
                state.message_count,
                state.transcript.len(),
                tools_count,
                skills_count,
                mcp_count,
                running_tasks,
            )),
        ]),
        Line::from({
            let mut line = Vec::new();
            let permissions_summary = session_permissions().get_summary();
            for item in permissions_summary {
                match item {
                    PermissionSummaryItem::Cwd(cwd) => {
                        line.push(Span::styled("cwd", theme.header_label_permissions_style()));
                        line.push(Span::raw(format!(" {}", cwd)));
                    }
                    PermissionSummaryItem::ExtraAllowDirs(items) => {
                        line.push(Span::styled(
                            "extra allow dirs",
                            theme.header_label_permissions_style(),
                        ));
                        line.push(Span::raw(format!(
                            " {}",
                            if items.is_empty() {
                                String::from("none")
                            } else {
                                items.join(", ")
                            }
                        )));
                    }
                    PermissionSummaryItem::DangerousAllowDirs(items) => {
                        line.push(Span::styled(
                            "dangerous allowlist",
                            theme.header_label_permissions_style(),
                        ));
                        line.push(Span::raw(format!(
                            " {}",
                            if items.is_empty() {
                                String::from("none")
                            } else {
                                items.join(", ")
                            }
                        )));
                    }
                }
                line.push(Span::raw("   "));
            }

            if !recent.is_empty() {
                line.push(Span::styled("recent", theme.header_label_recent_style()));
                line.push(Span::raw(format!(" {}", recent)));
            }

            line
        }),
    ]
}
