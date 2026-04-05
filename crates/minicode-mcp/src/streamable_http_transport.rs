use std::collections::HashMap;

use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

pub(crate) async fn start_streamable_http_service(
    url: &str,
    headers: &HashMap<String, String>,
    server_name: &str,
) -> anyhow::Result<RunningService<rmcp::RoleClient, ()>> {
    let mut custom_headers = HashMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            anyhow::anyhow!(
                "Invalid header name for MCP server {}: {} ({})",
                server_name,
                name,
                err
            )
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|err| {
            anyhow::anyhow!(
                "Invalid header value for MCP server {}: {} ({})",
                server_name,
                name,
                err
            )
        })?;
        custom_headers.insert(header_name, header_value);
    }

    let config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .custom_headers(custom_headers)
        .reinit_on_expired_session(true);

    let transport = StreamableHttpClientTransport::from_config(config);
    ().serve(transport).await.map_err(|err| {
        anyhow::anyhow!(
            "Failed to initialize remote MCP server {} via rmcp(streamable-http): {}",
            server_name,
            err
        )
    })
}
