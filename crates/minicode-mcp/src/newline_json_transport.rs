use std::process::Stdio;

use rmcp::ServiceExt;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;

pub(crate) async fn start_newline_json_service(
    command: tokio::process::Command,
    server_name: &str,
    command_display: &str,
) -> anyhow::Result<RunningService<rmcp::RoleClient, ()>> {
    let (transport, _stderr) = TokioChildProcess::builder(command)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            anyhow::anyhow!(
                "Failed to start MCP server {} with command {}: {}",
                server_name,
                command_display,
                err
            )
        })?;

    ().serve(transport).await.map_err(|err| {
        anyhow::anyhow!(
            "Failed to initialize MCP server {} via rmcp: {}",
            server_name,
            err
        )
    })
}
