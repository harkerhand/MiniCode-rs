use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use minicode_core::config::McpServerConfig;
use minicode_core::prompt::{McpServerSummary, SkillSummary};
use minicode_tool::{Tool, ToolContext, ToolRegistry, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// `npx` based MCP servers (e.g. server-filesystem) can take noticeably longer
// on first run while package resolution/download happens.
const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const MCP_INIT_TIMEOUT: Duration = Duration::from_secs(2);
const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(3);
static MCP_LOG_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_mcp_logging_enabled(enabled: bool) {
    MCP_LOG_ENABLED.store(enabled, Ordering::Relaxed);
}

fn mcp_log(message: impl AsRef<str>) {
    if !MCP_LOG_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    eprintln!("\x1b[32m[mcp]\x1b[0m {}", message.as_ref());
}

pub struct McpBundle {
    pub tools: Vec<Arc<dyn Tool>>,
    pub servers: Vec<McpServerSummary>,
    pub disposer: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonRpcProtocol {
    ContentLength,
    NewlineJson,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcMessage {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpToolDescriptor {
    name: String,
    description: Option<String>,
    #[serde(rename = "inputSchema")]
    input_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpResourceDescriptor {
    uri: String,
    name: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpPromptArg {
    name: String,
    required: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpPromptDescriptor {
    name: String,
    description: Option<String>,
    arguments: Option<Vec<McpPromptArg>>,
}

struct StdioMcpClient {
    server_name: String,
    process: Child,
    stdin: ChildStdin,
    responses: Receiver<anyhow::Result<JsonRpcMessage>>,
    reader_handle: Option<JoinHandle<()>>,
    protocol: JsonRpcProtocol,
    next_id: u64,
}

fn protocol_label(protocol: JsonRpcProtocol) -> &'static str {
    match protocol {
        JsonRpcProtocol::ContentLength => "content-length",
        JsonRpcProtocol::NewlineJson => "newline-json",
    }
}

impl StdioMcpClient {
    fn start(
        server_name: &str,
        config: &McpServerConfig,
        cwd: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let command = config.command.trim();
        if command.is_empty() {
            return Err(anyhow::anyhow!(
                "MCP server {} has empty command",
                server_name
            ));
        }

        let protocol_candidates = match config.protocol.as_deref() {
            Some("content-length") => vec![JsonRpcProtocol::ContentLength],
            Some("newline-json") => vec![JsonRpcProtocol::NewlineJson],
            _ => vec![JsonRpcProtocol::ContentLength, JsonRpcProtocol::NewlineJson],
        };

        let mut last_err = None;

        for protocol in protocol_candidates {
            mcp_log(format!(
                "server={} trying protocol={}",
                server_name,
                protocol_label(protocol)
            ));
            match Self::start_with_protocol(server_name, config, cwd, protocol) {
                Ok(mut client) => {
                    mcp_log(format!(
                        "server={} protocol={} spawned, sending initialize",
                        server_name,
                        protocol_label(protocol)
                    ));
                    let init = client.request_with_timeout(
                        "initialize",
                        json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "clientInfo": {
                                "name": "mini-code-rs",
                                "version": "0.1.0"
                            }
                        }),
                        MCP_INIT_TIMEOUT,
                    );
                    if let Err(err) = init {
                        mcp_log(format!(
                            "server={} protocol={} initialize failed: {}",
                            server_name,
                            protocol_label(protocol),
                            err
                        ));
                        let _ = client.close();
                        last_err = Some(err);
                        continue;
                    }
                    let _ = client.notify("notifications/initialized", json!({}));
                    mcp_log(format!(
                        "server={} protocol={} initialize ok",
                        server_name,
                        protocol_label(protocol)
                    ));
                    return Ok(client);
                }
                Err(err) => {
                    mcp_log(format!(
                        "server={} protocol={} spawn/start failed: {}",
                        server_name,
                        protocol_label(protocol),
                        err
                    ));
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Failed to start MCP server {server_name}")))
    }

    fn start_with_protocol(
        server_name: &str,
        config: &McpServerConfig,
        cwd: &std::path::Path,
        protocol: JsonRpcProtocol,
    ) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(config.args.clone().unwrap_or_default())
            .current_dir(if let Some(custom) = &config.cwd {
                cwd.join(custom)
            } else {
                cwd.to_path_buf()
            })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        mcp_log(format!(
            "server={} spawn command={} args={:?} cwd={}",
            server_name,
            config.command,
            config.args.clone().unwrap_or_default(),
            if let Some(custom) = &config.cwd {
                cwd.join(custom).display().to_string()
            } else {
                cwd.display().to_string()
            }
        ));

        if let Some(envs) = &config.env {
            for (k, v) in envs {
                cmd.env(k, v.to_string().trim_matches('"'));
            }
        }

        let mut child = cmd.spawn().map_err(|err| {
            anyhow::anyhow!(
                "Failed to start MCP server {} with command {}: {}",
                server_name,
                config.command,
                err
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture MCP stdin for {}", server_name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture MCP stdout for {}", server_name))?;
        let (tx, rx) = mpsc::channel();
        let reader_handle = Some(Self::spawn_reader_loop(
            server_name.to_string(),
            BufReader::new(stdout),
            protocol,
            tx,
        ));

        Ok(Self {
            server_name: server_name.to_string(),
            process: child,
            stdin,
            responses: rx,
            reader_handle,
            protocol,
            next_id: 1,
        })
    }

    fn notify(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        let msg = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: Some(method.to_string()),
            params: Some(params),
            result: None,
            error: None,
        };
        self.send(&msg)
    }

    fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.request_with_timeout(method, params, MCP_REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: Some(method.to_string()),
            params: Some(params),
            result: None,
            error: None,
        };

        self.send(&msg)?;
        mcp_log(format!(
            "server={} method={} request sent (timeout={}ms)",
            self.server_name,
            method,
            timeout.as_millis()
        ));

        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow::anyhow!(
                    "MCP {} request timed out for {}",
                    self.server_name,
                    method
                ));
            }
            let remaining = deadline.saturating_duration_since(now);
            let reply = match self.responses.recv_timeout(remaining) {
                Ok(Ok(message)) => message,
                Ok(Err(err)) => return Err(err),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(anyhow::anyhow!(
                        "MCP {} request timed out for {}",
                        self.server_name,
                        method
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(anyhow::anyhow!(
                        "MCP {} reader disconnected",
                        self.server_name
                    ));
                }
            };
            if reply.id != Some(id) {
                mcp_log(format!(
                    "server={} method={} got unrelated response id={:?}, waiting target id={}",
                    self.server_name, method, reply.id, id
                ));
                continue;
            }
            if let Some(err) = reply.error {
                return Err(anyhow::anyhow!(
                    "MCP {} error {}: {}",
                    self.server_name,
                    err.code,
                    err.message
                ));
            }
            mcp_log(format!(
                "server={} method={} request completed",
                self.server_name, method
            ));
            return Ok(reply.result.unwrap_or(Value::Null));
        }
    }

    fn send(&mut self, message: &JsonRpcMessage) -> anyhow::Result<()> {
        let body = serde_json::to_vec(message)?;
        match self.protocol {
            JsonRpcProtocol::NewlineJson => {
                self.stdin.write_all(&body)?;
                self.stdin.write_all(b"\n")?;
            }
            JsonRpcProtocol::ContentLength => {
                self.stdin
                    .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())?;
                self.stdin.write_all(&body)?;
            }
        }
        self.stdin.flush()?;
        Ok(())
    }

    fn spawn_reader_loop(
        server_name: String,
        mut stdout: BufReader<ChildStdout>,
        protocol: JsonRpcProtocol,
        tx: Sender<anyhow::Result<JsonRpcMessage>>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            loop {
                let result = Self::read_message_from(&server_name, &mut stdout, protocol);
                match result {
                    Ok(message) => {
                        if tx.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        break;
                    }
                }
            }
        })
    }

    fn read_message_from(
        server_name: &str,
        stdout: &mut BufReader<ChildStdout>,
        protocol: JsonRpcProtocol,
    ) -> anyhow::Result<JsonRpcMessage> {
        match protocol {
            JsonRpcProtocol::NewlineJson => {
                let mut line = String::new();
                stdout.read_line(&mut line)?;
                if line.is_empty() {
                    return Err(anyhow::anyhow!("MCP {} stdout EOF", server_name));
                }
                if line.trim().is_empty() {
                    return Err(anyhow::anyhow!(
                        "MCP {} returned empty JSON line",
                        server_name
                    ));
                }
                serde_json::from_str(line.trim()).map_err(|err| {
                    anyhow::anyhow!("MCP {} invalid JSON line: {}", server_name, err)
                })
            }
            JsonRpcProtocol::ContentLength => {
                let mut content_length = None::<usize>;
                loop {
                    let mut line = String::new();
                    stdout.read_line(&mut line)?;
                    if line.is_empty() {
                        return Err(anyhow::anyhow!(
                            "MCP {} stdout EOF while reading content-length headers",
                            server_name
                        ));
                    }
                    let trimmed = line.trim_end();
                    if content_length.is_none()
                        && (trimmed.starts_with('{') || trimmed.starts_with('['))
                    {
                        return Err(anyhow::anyhow!(
                            "MCP {} returned JSON payload before Content-Length header (likely newline-json protocol)",
                            server_name
                        ));
                    }
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = trimmed.split_once(':')
                        && name.trim().eq_ignore_ascii_case("content-length")
                    {
                        let v = value.trim();
                        content_length = Some(v.trim().parse::<usize>()?);
                    }
                }

                let len = content_length
                    .ok_or_else(|| anyhow::anyhow!("MCP {} missing content-length", server_name))?;
                let mut payload = vec![0u8; len];
                stdout.read_exact(&mut payload)?;
                serde_json::from_slice(&payload).map_err(|err| {
                    anyhow::anyhow!(
                        "MCP {} invalid content-length JSON payload: {}",
                        server_name,
                        err
                    )
                })
            }
        }
    }

    fn list_tools(&mut self) -> anyhow::Result<Vec<McpToolDescriptor>> {
        let result = self.request("tools/list", json!({}))?;
        Ok(result
            .get("tools")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn list_resources(&mut self) -> anyhow::Result<Vec<McpResourceDescriptor>> {
        let result = self.request_with_timeout("resources/list", json!({}), MCP_LIST_TIMEOUT)?;
        Ok(result
            .get("resources")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn list_prompts(&mut self) -> anyhow::Result<Vec<McpPromptDescriptor>> {
        let result = self.request_with_timeout("prompts/list", json!({}), MCP_LIST_TIMEOUT)?;
        Ok(result
            .get("prompts")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn call_tool(&mut self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
        let result = self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": input,
            }),
        )?;
        Ok(format_tool_result(result))
    }

    fn read_resource(&mut self, uri: &str) -> anyhow::Result<ToolResult> {
        let result = self.request("resources/read", json!({ "uri": uri }))?;
        Ok(ToolResult::ok(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()),
        ))
    }

    fn get_prompt(&mut self, name: &str, args: Value) -> anyhow::Result<ToolResult> {
        let result = self.request(
            "prompts/get",
            json!({
                "name": name,
                "arguments": args,
            }),
        )?;
        Ok(ToolResult::ok(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()),
        ))
    }

    fn close(&mut self) -> anyhow::Result<()> {
        mcp_log(format!("server={} closing process", self.server_name));
        let _ = self.process.kill();
        let _ = self.process.try_wait();
        let _ = self.reader_handle.take();
        mcp_log(format!("server={} closed", self.server_name));
        Ok(())
    }
}

fn sanitize_segment(value: &str) -> String {
    let mut s = value
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    s = s.trim_matches('_').to_string();
    if s.is_empty() { "tool".to_string() } else { s }
}

fn format_tool_result(result: Value) -> ToolResult {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut parts = vec![];
    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        for block in content {
            if block.get("type").and_then(|v| v.as_str()) == Some("text")
                && let Some(text) = block.get("text").and_then(|v| v.as_str())
            {
                parts.push(text.to_string());
                continue;
            }

            parts.push(serde_json::to_string_pretty(block).unwrap_or_else(|_| block.to_string()));
        }
    }
    if let Some(structured) = result.get("structuredContent") {
        parts.push(format!(
            "STRUCTURED_CONTENT:\n{}",
            serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string())
        ));
    }
    if parts.is_empty() {
        parts.push(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()));
    }

    ToolResult {
        ok: !is_error,
        output: parts.join("\n\n"),
        background_task: None,
        await_user: false,
    }
}

struct McpDynamicTool {
    wrapped_name: String,
    description: String,
    input_schema: Value,
    tool_name: String,
    client: Arc<Mutex<StdioMcpClient>>,
}

#[async_trait]
impl Tool for McpDynamicTool {
    fn name(&self) -> &str {
        &self.wrapped_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let Ok(mut client) = self.client.lock() else {
            return ToolResult::err("Failed to lock MCP client");
        };
        match client.call_tool(&self.tool_name, input) {
            Ok(result) => result,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

struct ListMcpResourcesTool {
    entries: Vec<(String, McpResourceDescriptor)>,
}

#[async_trait]
impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "list_mcp_resources"
    }

    fn description(&self) -> &str {
        "列出当前已连接 MCP 服务提供的资源。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"}}})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server_filter = input.get("server").and_then(|v| v.as_str());
        let lines = self
            .entries
            .iter()
            .filter(|(server, _)| match server_filter {
                Some(f) => f == server,
                None => true,
            })
            .map(|(server, resource)| {
                format!(
                    "{}: {}{}{}",
                    server,
                    resource.uri,
                    resource
                        .name
                        .as_ref()
                        .map(|x| format!(" ({})", x))
                        .unwrap_or_default(),
                    resource
                        .description
                        .as_ref()
                        .map(|x| format!(" - {}", x))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();

        if lines.is_empty() {
            ToolResult::ok("No MCP resources available.")
        } else {
            ToolResult::ok(lines.join("\n"))
        }
    }
}

struct ReadMcpResourceTool {
    clients: HashMap<String, Arc<Mutex<StdioMcpClient>>>,
}

#[async_trait]
impl Tool for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "read_mcp_resource"
    }

    fn description(&self) -> &str {
        "读取指定 MCP 资源。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"},"uri":{"type":"string"}},"required":["server","uri"]})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server = input
            .get("server")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let uri = input
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if server.is_empty() || uri.is_empty() {
            return ToolResult::err("server/uri is required");
        }
        let Some(client) = self.clients.get(server) else {
            return ToolResult::err(format!("Unknown MCP server: {}", server));
        };
        let Ok(mut inner) = client.lock() else {
            return ToolResult::err("Failed to lock MCP client");
        };
        match inner.read_resource(uri) {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

struct ListMcpPromptsTool {
    entries: Vec<(String, McpPromptDescriptor)>,
}

#[async_trait]
impl Tool for ListMcpPromptsTool {
    fn name(&self) -> &str {
        "list_mcp_prompts"
    }

    fn description(&self) -> &str {
        "列出当前已连接 MCP 服务提供的提示模板。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"}}})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server_filter = input.get("server").and_then(|v| v.as_str());
        let lines = self
            .entries
            .iter()
            .filter(|(server, _)| match server_filter {
                Some(f) => f == server,
                None => true,
            })
            .map(|(server, prompt)| {
                let args = prompt
                    .arguments
                    .as_ref()
                    .map(|x| {
                        x.iter()
                            .map(|a| {
                                format!(
                                    "{}{}",
                                    a.name,
                                    if a.required == Some(true) { "*" } else { "" }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!(
                    "{}: {}{}{}",
                    server,
                    prompt.name,
                    if args.is_empty() {
                        "".to_string()
                    } else {
                        format!(" args=[{}]", args)
                    },
                    prompt
                        .description
                        .as_ref()
                        .map(|x| format!(" - {}", x))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            ToolResult::ok("No MCP prompts available.")
        } else {
            ToolResult::ok(lines.join("\n"))
        }
    }
}

struct GetMcpPromptTool {
    clients: HashMap<String, Arc<Mutex<StdioMcpClient>>>,
}

#[async_trait]
impl Tool for GetMcpPromptTool {
    fn name(&self) -> &str {
        "get_mcp_prompt"
    }

    fn description(&self) -> &str {
        "渲染并获取 MCP Prompt。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"server":{"type":"string"},"name":{"type":"string"},"arguments":{"type":"object","additionalProperties":{"type":"string"}}},"required":["server","name"]})
    }

    async fn run(&self, input: Value, _context: &ToolContext) -> ToolResult {
        let server = input
            .get("server")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if server.is_empty() || name.is_empty() {
            return ToolResult::err("server/name is required");
        }

        let args = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let Some(client) = self.clients.get(server) else {
            return ToolResult::err(format!("Unknown MCP server: {}", server));
        };
        let Ok(mut inner) = client.lock() else {
            return ToolResult::err("Failed to lock MCP client");
        };
        match inner.get_prompt(name, args) {
            Ok(v) => v,
            Err(err) => ToolResult::err(err.to_string()),
        }
    }
}

pub async fn create_mcp_backed_tools(
    cwd: &std::path::Path,
    mcp_servers: &HashMap<String, McpServerConfig>,
) -> McpBundle {
    let mut tools: Vec<Arc<dyn Tool>> = vec![];
    let mut servers = vec![];
    let mut clients: HashMap<String, Arc<Mutex<StdioMcpClient>>> = HashMap::new();
    let mut resource_entries: Vec<(String, McpResourceDescriptor)> = vec![];
    let mut prompt_entries: Vec<(String, McpPromptDescriptor)> = vec![];
    let mut closers: Vec<Arc<Mutex<StdioMcpClient>>> = vec![];

    struct ConnectSuccess {
        server_name: String,
        config: McpServerConfig,
        client: StdioMcpClient,
        tool_descriptors: Vec<McpToolDescriptor>,
        resources: Vec<McpResourceDescriptor>,
        prompts: Vec<McpPromptDescriptor>,
        protocol: String,
    }

    enum ConnectOutcome {
        Success(ConnectSuccess),
        Failure {
            server_name: String,
            config: McpServerConfig,
            error: String,
        },
    }

    let mut pending = FuturesUnordered::new();

    for (server_name, config) in mcp_servers {
        mcp_log(format!("bootstrap begin server={}", server_name));
        if config.enabled == Some(false) {
            servers.push(McpServerSummary {
                name: server_name.clone(),
                command: config.command.clone(),
                status: "disabled".to_string(),
                tool_count: 0,
                error: None,
                protocol: config.protocol.clone(),
                resource_count: Some(0),
                prompt_count: Some(0),
            });
            continue;
        }

        let server_name = server_name.clone();
        let config = config.clone();
        let cwd = cwd.to_path_buf();
        pending.push(async move {
            let join = tokio::task::spawn_blocking({
                let server_name = server_name.clone();
                let config = config.clone();
                move || -> anyhow::Result<ConnectSuccess> {
                    let mut client = StdioMcpClient::start(&server_name, &config, &cwd)?;
                    let protocol = protocol_label(client.protocol).to_string();
                    mcp_log(format!(
                        "bootstrap server={} connected protocol={}, listing tools/resources/prompts",
                        server_name, protocol
                    ));
                    let tool_descriptors = client.list_tools().unwrap_or_default();
                    let resources = client.list_resources().unwrap_or_default();
                    let prompts = client.list_prompts().unwrap_or_default();
                    mcp_log(format!(
                        "bootstrap server={} listed tools={}, resources={}, prompts={}",
                        server_name,
                        tool_descriptors.len(),
                        resources.len(),
                        prompts.len()
                    ));
                    Ok(ConnectSuccess {
                        server_name,
                        config,
                        client,
                        tool_descriptors,
                        resources,
                        prompts,
                        protocol,
                    })
                }
            });

            match tokio::time::timeout(MCP_STARTUP_TIMEOUT, join).await {
                Ok(Ok(Ok(success))) => ConnectOutcome::Success(success),
                Ok(Ok(Err(err))) => ConnectOutcome::Failure {
                    server_name,
                    config,
                    error: err.to_string(),
                },
                Ok(Err(err)) => ConnectOutcome::Failure {
                    server_name,
                    config,
                    error: format!("MCP startup task failed: {err}"),
                },
                Err(_) => ConnectOutcome::Failure {
                    server_name,
                    config,
                    error: format!(
                        "MCP startup timed out after {}s (try pre-installing the server package or increasing network speed)",
                        MCP_STARTUP_TIMEOUT.as_secs(),
                    ),
                },
            }
        });
    }

    while let Some(outcome) = pending.next().await {
        match outcome {
            ConnectOutcome::Success(success) => {
                mcp_log(format!(
                    "bootstrap success server={} tools={} resources={} prompts={}",
                    success.server_name,
                    success.tool_descriptors.len(),
                    success.resources.len(),
                    success.prompts.len()
                ));
                let server_name = success.server_name;
                let config = success.config;
                let tool_descriptors = success.tool_descriptors;
                let resources = success.resources;
                let prompts = success.prompts;
                let client = Arc::new(Mutex::new(success.client));

                clients.insert(server_name.clone(), client.clone());
                closers.push(client.clone());

                for descriptor in &tool_descriptors {
                    let wrapped_name = format!(
                        "mcp__{}__{}",
                        sanitize_segment(&server_name),
                        sanitize_segment(&descriptor.name)
                    );
                    tools.push(Arc::new(McpDynamicTool {
                        wrapped_name,
                        description: descriptor.description.clone().unwrap_or_else(|| {
                            format!(
                                "Call MCP tool {} from server {}.",
                                descriptor.name, server_name
                            )
                        }),
                        input_schema: descriptor.input_schema.clone().unwrap_or_else(
                            || json!({"type":"object","additionalProperties":true}),
                        ),
                        tool_name: descriptor.name.clone(),
                        client: client.clone(),
                    }));
                }

                for resource in resources.clone() {
                    resource_entries.push((server_name.clone(), resource));
                }
                for prompt in prompts.clone() {
                    prompt_entries.push((server_name.clone(), prompt));
                }

                servers.push(McpServerSummary {
                    name: server_name,
                    command: config.command,
                    status: "connected".to_string(),
                    tool_count: tool_descriptors.len(),
                    error: None,
                    protocol: Some(success.protocol),
                    resource_count: Some(resources.len()),
                    prompt_count: Some(prompts.len()),
                });
            }
            ConnectOutcome::Failure {
                server_name,
                config,
                error,
            } => {
                mcp_log(format!(
                    "bootstrap failure server={} error={}",
                    server_name, error
                ));
                servers.push(McpServerSummary {
                    name: server_name,
                    command: config.command,
                    status: "error".to_string(),
                    tool_count: 0,
                    error: Some(error),
                    protocol: config.protocol,
                    resource_count: Some(0),
                    prompt_count: Some(0),
                });
            }
        }
    }

    if !resource_entries.is_empty() {
        tools.push(Arc::new(ListMcpResourcesTool {
            entries: resource_entries,
        }));
        tools.push(Arc::new(ReadMcpResourceTool {
            clients: clients.clone(),
        }));
    }
    if !prompt_entries.is_empty() {
        tools.push(Arc::new(ListMcpPromptsTool {
            entries: prompt_entries,
        }));
        tools.push(Arc::new(GetMcpPromptTool {
            clients: clients.clone(),
        }));
    }

    let disposer = if closers.is_empty() {
        None
    } else {
        let closers = Arc::new(closers);
        Some(Arc::new(move || {
            let closers = closers.clone();
            let fut: BoxFuture<'static, ()> = Box::pin(async move {
                for client in closers.iter() {
                    if let Ok(mut inner) = client.lock() {
                        let _ = inner.close();
                    }
                }
            });
            fut
        })
            as Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>)
    };

    McpBundle {
        tools,
        servers,
        disposer,
    }
}

pub fn extend_registry_with_mcp(
    tools: Vec<Arc<dyn Tool>>,
    skills: Vec<SkillSummary>,
    mcp: McpBundle,
) -> ToolRegistry {
    let mut merged = tools;
    merged.extend(mcp.tools);
    ToolRegistry::new(merged, skills, mcp.servers, mcp.disposer)
}
