use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rmcp::RoleClient;
use rmcp::ServiceExt;
use rmcp::service::RunningService;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) struct ContentLengthChildProcessTransport {
    child: Option<Child>,
    stdout: BufReader<ChildStdout>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

impl ContentLengthChildProcessTransport {
    pub(crate) fn new(mut command: Command) -> io::Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("stdout was not piped"))?;

        Ok(Self {
            child: Some(child),
            stdout: BufReader::new(stdout),
            stdin: Arc::new(Mutex::new(Some(stdin))),
        })
    }

    async fn read_payload(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None);
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }

            if let Some((name, value)) = trimmed.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                let len = value.trim().parse::<usize>().map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid content-length value: {err}"),
                    )
                })?;
                content_length = Some(len);
            }
        }

        let len = content_length
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing content-length"))?;
        let mut payload = vec![0u8; len];
        self.stdout.read_exact(&mut payload).await?;
        Ok(Some(payload))
    }
}

pub(crate) async fn start_content_length_service(
    command: tokio::process::Command,
    server_name: &str,
    command_display: &str,
) -> anyhow::Result<RunningService<rmcp::RoleClient, ()>> {
    let transport = ContentLengthChildProcessTransport::new(command).map_err(|err| {
        anyhow::anyhow!(
            "Failed to start MCP server {} with command {} (content-length): {}",
            server_name,
            command_display,
            err
        )
    })?;

    ().serve(transport).await.map_err(|err| {
        anyhow::anyhow!(
            "Failed to initialize MCP server {} via rmcp(content-length): {}",
            server_name,
            err
        )
    })
}

impl Transport<RoleClient> for ContentLengthChildProcessTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let stdin = self.stdin.clone();
        async move {
            let body = serde_json::to_vec(&item).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("serialize json-rpc: {err}"),
                )
            })?;
            let mut guard = stdin.lock().await;
            let writer = guard
                .as_mut()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "stdin is closed"))?;
            writer
                .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
                .await?;
            writer.write_all(&body).await?;
            writer.flush().await?;
            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let payload = match self.read_payload().await {
            Ok(Some(payload)) => payload,
            Ok(None) => return None,
            Err(_) => return None,
        };
        serde_json::from_slice::<RxJsonRpcMessage<RoleClient>>(&payload).ok()
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        {
            let mut guard = self.stdin.lock().await;
            *guard = None;
        }

        if let Some(mut child) = self.child.take() {
            match tokio::time::timeout(SHUTDOWN_TIMEOUT, child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => return Err(err),
                Err(_) => {
                    child.kill().await?;
                    let _ = child.wait().await;
                }
            }
        }

        Ok(())
    }
}
