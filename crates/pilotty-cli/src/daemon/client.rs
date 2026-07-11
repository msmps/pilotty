//! Client for connecting to the daemon process.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use pilotty_core::error::ApiError;
use pilotty_core::protocol::{
    supports_protocol, Command, Request, Response, LEGACY_PROTOCOL_VERSION, PROTOCOL_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;
use tracing::{debug, info};

use crate::daemon::paths;

/// Maximum time to wait for daemon to start up.
const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval between socket connection attempts.
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// Client for communicating with the daemon.
pub struct DaemonClient {
    stream: UnixStream,
    daemon_protocol: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolAction {
    Send,
    Probe,
    Reject { observed: u32, required: u32 },
}

fn protocol_action(observed: Option<u32>, required: u32) -> ProtocolAction {
    if required == LEGACY_PROTOCOL_VERSION {
        return ProtocolAction::Send;
    }

    match observed {
        None => ProtocolAction::Probe,
        Some(observed) if supports_protocol(observed, required) => ProtocolAction::Send,
        Some(observed) => ProtocolAction::Reject { observed, required },
    }
}

impl DaemonClient {
    /// Connect to the daemon, starting it if necessary.
    pub async fn connect() -> Result<Self> {
        let socket_path = paths::get_socket_path(None);

        // Try to connect directly first
        if let Ok(stream) = UnixStream::connect(&socket_path).await {
            debug!("Connected to existing daemon");
            return Ok(Self {
                stream,
                daemon_protocol: None,
            });
        }

        // Daemon not running, start it
        info!("Daemon not running, starting...");
        let child = Self::start_daemon()?;

        // Wait for daemon to become available, checking if it crashes
        let stream = Self::wait_for_daemon(&socket_path, child).await?;
        Ok(Self {
            stream,
            daemon_protocol: None,
        })
    }

    /// Start the daemon as a background process.
    ///
    /// Returns the child process handle so we can detect early crashes.
    fn start_daemon() -> Result<std::process::Child> {
        use std::os::unix::process::CommandExt;

        let exe = std::env::current_exe().context("Failed to get current executable path")?;

        // Spawn daemon as detached background process.
        // process_group(0) creates a new process group with the child as leader,
        // preventing the daemon from receiving SIGHUP when the CLI's terminal closes.
        let child = std::process::Command::new(exe)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .context("Failed to spawn daemon process")?;

        Ok(child)
    }

    /// Wait for the daemon socket to become available.
    ///
    /// Checks if the daemon process crashes early to provide a faster error
    /// instead of waiting for the full timeout.
    async fn wait_for_daemon(
        socket_path: &PathBuf,
        mut child: std::process::Child,
    ) -> Result<UnixStream> {
        let start = std::time::Instant::now();

        loop {
            // Check if daemon crashed before we could connect
            match child.try_wait() {
                Ok(Some(status)) => {
                    bail!(
                        "Daemon exited immediately with status: {} (check logs or run 'pilotty daemon' directly to diagnose)",
                        status
                    );
                }
                Ok(None) => {
                    // Still running, continue trying to connect
                }
                Err(e) => {
                    // Error checking status, log but continue
                    debug!("Error checking daemon status: {}", e);
                }
            }

            match UnixStream::connect(socket_path).await {
                Ok(stream) => {
                    info!("Connected to daemon after {:?}", start.elapsed());
                    return Ok(stream);
                }
                Err(_) => {
                    if start.elapsed() > DAEMON_STARTUP_TIMEOUT {
                        bail!("Daemon failed to start within {:?}", DAEMON_STARTUP_TIMEOUT);
                    }
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            }
        }
    }

    /// Send a request and wait for a response.
    pub async fn request(&mut self, request: Request) -> Result<Response> {
        self.request_with_timeout(request, Duration::from_secs(30))
            .await
    }

    /// Send a request and wait for a response with a custom timeout.
    pub async fn request_with_timeout(
        &mut self,
        request: Request,
        timeout_duration: Duration,
    ) -> Result<Response> {
        let required = request.command.minimum_protocol();
        self.ensure_protocol(&request.id, required, timeout_duration)
            .await?;

        let response = self.exchange(request, timeout_duration).await?;
        self.daemon_protocol = Some(response.protocol);

        if response.protocol < PROTOCOL_VERSION {
            eprintln!(
                "warning: the running daemon speaks protocol {} but this client speaks {}; \
                 run 'pilotty stop' so the daemon restarts with the current binary",
                response.protocol, PROTOCOL_VERSION
            );
        }

        Ok(response)
    }

    async fn ensure_protocol(
        &mut self,
        request_id: &str,
        required: u32,
        timeout_duration: Duration,
    ) -> Result<()> {
        loop {
            match protocol_action(self.daemon_protocol, required) {
                ProtocolAction::Send => return Ok(()),
                ProtocolAction::Probe => {
                    let probe = Request::new(
                        format!("{request_id}-protocol-probe"),
                        Command::ListSessions,
                    );
                    let response = self.exchange(probe, timeout_duration).await?;
                    self.daemon_protocol = Some(response.protocol);
                }
                ProtocolAction::Reject { observed, required } => {
                    return Err(ApiError::protocol_upgrade_required(observed, required).into());
                }
            }
        }
    }

    async fn exchange(&mut self, request: Request, timeout_duration: Duration) -> Result<Response> {
        let request_json =
            serde_json::to_string(&request).context("Failed to serialize request")?;
        debug!("Sending: {}", request_json);

        // Send request
        self.stream
            .write_all(request_json.as_bytes())
            .await
            .context("Failed to write request")?;
        self.stream
            .write_all(b"\n")
            .await
            .context("Failed to write newline")?;
        self.stream.flush().await.context("Failed to flush")?;

        // Read response with timeout
        let (reader, _writer) = self.stream.split();
        let mut reader = BufReader::new(reader);
        let mut response_line = String::new();

        let bytes_read = timeout(timeout_duration, reader.read_line(&mut response_line))
            .await
            .context("Request timed out")?
            .context("Failed to read response")?;

        if bytes_read == 0 {
            bail!("Daemon closed connection unexpectedly");
        }

        debug!("Received: {}", response_line.trim());

        let response: Response =
            serde_json::from_str(&response_line).context("Failed to parse response")?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::server::DaemonServer;
    use pilotty_core::protocol::{Command, ResponseData, PROTOCOL_V1, PROTOCOL_VERSION};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn test_client_connects_to_running_daemon() {
        // Use a temp socket path
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-client-test-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        // Start server
        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(2), server.run()).await;
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect client directly (bypassing auto-start since we're using temp socket)
        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let mut client = DaemonClient {
            stream,
            daemon_protocol: None,
        };

        // Send request
        let request = Request {
            protocol: PROTOCOL_VERSION,
            id: "client-test-1".to_string(),
            command: Command::ListSessions,
        };

        let response = client.request(request).await.expect("Request failed");
        assert!(response.success);
        assert_eq!(response.id, "client-test-1");

        // Clean up
        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn unknown_peer_is_probed_only_for_versioned_commands() {
        assert_eq!(protocol_action(None, 0), ProtocolAction::Send);
        assert_eq!(
            protocol_action(None, PROTOCOL_VERSION),
            ProtocolAction::Probe
        );
    }

    #[test]
    fn observed_peer_protocol_decides_versioned_command_support() {
        assert_eq!(
            protocol_action(Some(0), PROTOCOL_VERSION),
            ProtocolAction::Reject {
                observed: 0,
                required: PROTOCOL_VERSION,
            }
        );
        assert_eq!(
            protocol_action(Some(PROTOCOL_VERSION), PROTOCOL_VERSION),
            ProtocolAction::Send
        );
    }

    #[tokio::test]
    async fn shipped_v1_daemon_is_probed_and_current_command_is_rejected_before_transmission() {
        let socket_path =
            std::env::temp_dir().join(format!("pilotty-probe-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("bind protocol fixture");
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept client");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read probe");
            let probe: Request = serde_json::from_str(&line).expect("parse probe");
            assert_eq!(probe.command, Command::ListSessions);

            let response = Response {
                id: probe.id,
                success: true,
                data: Some(ResponseData::Sessions { sessions: vec![] }),
                error: None,
                protocol: PROTOCOL_V1,
            };
            writer
                .write_all(
                    serde_json::to_string(&response)
                        .expect("serialize response")
                        .as_bytes(),
                )
                .await
                .expect("write response");
            writer.write_all(b"\n").await.expect("write newline");
            writer.flush().await.expect("flush response");

            line.clear();
            assert!(
                timeout(Duration::from_millis(100), reader.read_line(&mut line))
                    .await
                    .is_err(),
                "version-gated command must not reach an old daemon"
            );
        });

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("connect to protocol fixture");
        let mut client = DaemonClient {
            stream,
            daemon_protocol: None,
        };
        let error = client
            .request(Request::new(
                "logs-request",
                Command::Logs {
                    session: None,
                    ansi: false,
                },
            ))
            .await
            .expect_err("old daemon must be rejected");

        let message = error.to_string();
        assert!(message.contains("speaks protocol 1"), "got: {message}");
        assert!(
            message.contains(&format!("requires protocol {PROTOCOL_VERSION}")),
            "got: {message}"
        );
        peer.await.expect("protocol fixture task");
        let _ = std::fs::remove_file(&socket_path);
    }
}
