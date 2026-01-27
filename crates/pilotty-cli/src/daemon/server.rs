//! Unix socket server for the daemon process.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pilotty_core::error::ApiError;
use pilotty_core::protocol::{Command, Request, Response, ResponseData, SnapshotFormat};
use pilotty_core::snapshot::{CursorState, ScreenState, TerminalSize};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::daemon::paths;
use crate::daemon::session::{SessionId, SessionManager};

/// Maximum number of concurrent client connections to prevent resource exhaustion.
const MAX_CONNECTIONS: usize = 100;

/// How long the daemon waits with no sessions before auto-shutdown (5 minutes).
const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// How often to check for idle shutdown condition.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// How long to wait for in-flight connections to complete during shutdown.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// The daemon server that listens for client connections.
pub struct DaemonServer {
    listener: UnixListener,
    socket_path: PathBuf,
    pid_path: PathBuf,
    sessions: Arc<SessionManager>,
    /// Semaphore to limit concurrent connections and prevent resource exhaustion.
    connection_semaphore: Arc<Semaphore>,
    /// Shutdown signal for graceful termination (allows Drop to run and clean up files).
    shutdown: Arc<Notify>,
}

impl DaemonServer {
    /// Create a new daemon server bound to the default socket path.
    pub async fn bind() -> Result<Self> {
        let socket_path = paths::get_socket_path(None);
        let pid_path = paths::get_pid_path(None);
        Self::bind_to(socket_path, pid_path).await
    }

    /// Create a new daemon server bound to a specific socket path.
    ///
    /// Uses a bind-first approach to avoid TOCTOU race conditions:
    /// 1. Try to bind directly
    /// 2. If socket in use, check PID file to see if daemon is alive
    /// 3. If daemon dead, remove stale socket and retry
    /// 4. If daemon alive, return error
    pub async fn bind_to(socket_path: PathBuf, pid_path: PathBuf) -> Result<Self> {
        // Ensure socket directory exists with secure permissions (0700)
        paths::ensure_socket_dir().context("Failed to create socket directory")?;

        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create socket directory for {:?}", socket_path)
            })?;
        }

        // Helper to write PID file immediately after successful bind.
        // This closes the race window where another process could see our socket
        // but not find a valid PID file, incorrectly assuming we're dead.
        let write_pid = |pid_path: &PathBuf| -> Result<()> {
            std::fs::write(pid_path, std::process::id().to_string())
                .with_context(|| format!("Failed to write PID file: {:?}", pid_path))
        };

        // Try to bind directly (avoid TOCTOU race)
        let listener = match UnixListener::bind(&socket_path) {
            Ok(l) => {
                // Write PID immediately after bind to prevent race condition
                write_pid(&pid_path)?;
                l
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                // Socket exists, check if daemon is still alive
                if is_daemon_alive(&pid_path) {
                    anyhow::bail!(
                        "Daemon already running (socket {:?} in use, PID file valid)",
                        socket_path
                    );
                }

                // Daemon is dead, but verify the socket file is safe to remove
                // Don't follow symlinks (could delete unintended files)
                let metadata = std::fs::symlink_metadata(&socket_path)
                    .with_context(|| format!("Failed to stat socket path: {:?}", socket_path))?;

                if metadata.file_type().is_symlink() {
                    anyhow::bail!(
                        "Socket path {:?} is a symlink, refusing to delete for safety",
                        socket_path
                    );
                }

                // On Unix, verify it's actually a socket file
                #[cfg(unix)]
                {
                    use std::os::unix::fs::FileTypeExt;
                    if !metadata.file_type().is_socket() {
                        anyhow::bail!(
                            "Path {:?} exists but is not a socket file (type: {:?})",
                            socket_path,
                            metadata.file_type()
                        );
                    }
                }

                // Safe to remove stale socket
                info!("Removing stale socket from dead daemon");
                std::fs::remove_file(&socket_path)
                    .with_context(|| format!("Failed to remove stale socket: {:?}", socket_path))?;

                let l = UnixListener::bind(&socket_path)
                    .with_context(|| format!("Failed to bind to socket: {:?}", socket_path))?;
                // Write PID immediately after bind to prevent race condition
                write_pid(&pid_path)?;
                l
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Failed to bind to socket: {:?}", socket_path));
            }
        };

        info!("Daemon listening on {:?}", socket_path);

        Ok(Self {
            listener,
            socket_path,
            pid_path,
            sessions: Arc::new(SessionManager::new()),
            connection_semaphore: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            shutdown: Arc::new(Notify::new()),
        })
    }

    /// Run the server, accepting connections and handling requests.
    ///
    /// Limits concurrent connections via semaphore to prevent resource exhaustion.
    /// Spawns background tasks for:
    /// - Session cleaner: removes sessions when their child process exits
    /// - Idle shutdown: signals shutdown after 5 minutes with no sessions
    ///
    /// On shutdown, waits for in-flight connections to complete (with timeout).
    /// Returns when shutdown is signaled, allowing Drop to clean up socket/PID files.
    pub async fn run(&self) -> Result<()> {
        // Spawn session cleaner to remove dead sessions
        self.sessions.spawn_cleaner();

        // Spawn idle shutdown monitor
        self.spawn_idle_shutdown_task();

        // Track spawned connection handlers for graceful shutdown
        let mut connection_tasks: JoinSet<()> = JoinSet::new();

        loop {
            tokio::select! {
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            // Acquire a permit before spawning the connection handler.
                            // This limits concurrent connections to MAX_CONNECTIONS.
                            let permit = match self.connection_semaphore.clone().try_acquire_owned() {
                                Ok(permit) => permit,
                                Err(_) => {
                                    warn!(
                                        "Connection limit ({}) reached, rejecting new connection",
                                        MAX_CONNECTIONS
                                    );
                                    // Drop the stream to close the connection
                                    drop(stream);
                                    continue;
                                }
                            };

                            debug!("Accepted new connection");
                            let sessions = self.sessions.clone();
                            let shutdown = self.shutdown.clone();
                            connection_tasks.spawn(async move {
                                // Permit is held for the lifetime of the connection handler
                                let _permit = permit;
                                if let Err(e) = handle_connection(stream, sessions, shutdown).await {
                                    error!("Connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
                // Reap completed connection tasks to prevent unbounded growth
                Some(_) = connection_tasks.join_next(), if !connection_tasks.is_empty() => {
                    // Task completed, nothing to do (errors logged in handler)
                }
                _ = self.shutdown.notified() => {
                    info!("Shutdown signal received, waiting for in-flight connections");
                    break;
                }
            }
        }

        // Graceful shutdown: wait for in-flight connections with timeout
        if !connection_tasks.is_empty() {
            let pending = connection_tasks.len();
            info!(
                "Waiting for {} in-flight connection(s) to complete",
                pending
            );

            let shutdown_deadline = tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, async {
                while connection_tasks.join_next().await.is_some() {
                    // Keep draining until all tasks complete
                }
            })
            .await;

            if shutdown_deadline.is_err() {
                let remaining = connection_tasks.len();
                warn!(
                    "Graceful shutdown timed out after {:?}, aborting {} connection(s)",
                    GRACEFUL_SHUTDOWN_TIMEOUT, remaining
                );
                // JoinSet::abort_all() will cancel remaining tasks
                connection_tasks.abort_all();
            }
        }

        Ok(())
    }

    /// Spawn a background task that monitors for idle shutdown.
    ///
    /// The daemon will exit after IDLE_TIMEOUT (5 minutes) with no active sessions
    /// AND no active client connections. This prevents shutting down while a client
    /// is connected but hasn't spawned a session yet.
    ///
    /// Signals shutdown via Notify instead of calling exit(), allowing Drop to run.
    fn spawn_idle_shutdown_task(&self) {
        let sessions = self.sessions.clone();
        let shutdown = self.shutdown.clone();
        let semaphore = self.connection_semaphore.clone();

        tokio::spawn(async move {
            let mut idle_since: Option<Instant> = None;

            loop {
                tokio::time::sleep(IDLE_CHECK_INTERVAL).await;

                // Check both sessions AND active connections.
                // A client might be connected but not have spawned a session yet.
                let has_sessions = !sessions.is_empty().await;
                let has_connections = semaphore.available_permits() < MAX_CONNECTIONS;

                if has_sessions || has_connections {
                    // Activity detected, reset idle timer
                    if idle_since.is_some() {
                        if has_sessions {
                            debug!("Session activity detected, resetting idle timer");
                        } else {
                            debug!("Active connection detected, resetting idle timer");
                        }
                    }
                    idle_since = None;
                    continue;
                }

                // Truly idle: no sessions and no connections
                let idle_start = *idle_since.get_or_insert_with(Instant::now);

                if idle_start.elapsed() >= IDLE_TIMEOUT {
                    // Double-check to narrow race window
                    let still_has_sessions = !sessions.is_empty().await;
                    let still_has_connections = semaphore.available_permits() < MAX_CONNECTIONS;

                    if still_has_sessions || still_has_connections {
                        debug!("Activity detected during shutdown check, aborting shutdown");
                        idle_since = None;
                        continue;
                    }

                    info!(
                        "No activity for {} seconds, shutting down",
                        IDLE_TIMEOUT.as_secs()
                    );

                    // Kill any remaining sessions (defensive, should be none)
                    kill_all_sessions(&sessions).await;

                    // Signal main loop to exit (allows Drop to clean up files)
                    shutdown.notify_waiters();
                    break;
                }

                debug!(
                    "Idle for {} seconds (shutdown in {} seconds)",
                    idle_start.elapsed().as_secs(),
                    IDLE_TIMEOUT.saturating_sub(idle_start.elapsed()).as_secs()
                );
            }
        });
    }
}

/// Kill all active sessions during shutdown.
///
/// Used by both the shutdown command handler and the idle shutdown task.
async fn kill_all_sessions(sessions: &SessionManager) {
    let session_ids: Vec<_> = sessions
        .list_sessions()
        .await
        .iter()
        .map(|s| s.id.clone())
        .collect();

    for id in session_ids {
        let session_id = SessionId::from(id);
        if let Err(e) = sessions.kill_session(&session_id).await {
            warn!(
                "Failed to kill session {} during shutdown: {}",
                session_id, e
            );
        }
    }
}

impl Drop for DaemonServer {
    fn drop(&mut self) {
        // Clean up socket file on shutdown
        if self.socket_path.exists() && std::fs::remove_file(&self.socket_path).is_err() {
            warn!("Failed to remove socket on shutdown");
        }
        // Clean up PID file on shutdown
        if self.pid_path.exists() && std::fs::remove_file(&self.pid_path).is_err() {
            warn!("Failed to remove PID file on shutdown");
        }
    }
}

/// Check if a daemon process is still alive by reading its PID file.
///
/// Returns true if:
/// - PID file exists and contains a valid PID
/// - AND that process is still running (verified via kill(pid, 0))
fn is_daemon_alive(pid_path: &Path) -> bool {
    let pid_str = match std::fs::read_to_string(pid_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };

    // kill(pid, 0) checks if process exists without sending a signal.
    // SAFETY: libc::kill with signal 0 is a POSIX-defined no-op that only checks
    // whether the process exists and the caller has permission to signal it.
    // The pid is validated as a valid i32 above. No actual signal is delivered.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Maximum request size in bytes (1 MB should be plenty for any reasonable request).
const MAX_REQUEST_SIZE: usize = 1024 * 1024;

/// Maximum scroll amount to prevent long-running requests.
const MAX_SCROLL_AMOUNT: u32 = 1000;

/// Read a line with a maximum size limit to prevent memory DoS.
///
/// Returns the number of bytes read (0 means EOF).
/// Returns an error if the line exceeds max_size before finding a newline.
async fn read_line_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut String,
    max_size: usize,
) -> Result<usize> {
    use tokio::io::AsyncBufReadExt;

    let mut total = 0;
    let mut bytes = Vec::new();

    loop {
        let available = reader
            .fill_buf()
            .await
            .context("Failed to read from client")?;

        if available.is_empty() {
            // EOF
            if !bytes.is_empty() {
                let line = std::str::from_utf8(&bytes).context("Invalid UTF-8 in request")?;
                buf.push_str(line);
            }
            return Ok(total);
        }

        // Find newline in available data
        let newline_pos = available.iter().position(|&b| b == b'\n');
        let bytes_to_consume = newline_pos.map(|p| p + 1).unwrap_or(available.len());

        // Check size limit before consuming
        if total + bytes_to_consume > max_size {
            anyhow::bail!("Request too large: exceeded {} byte limit", max_size);
        }

        // Append raw bytes and validate UTF-8 once at the end
        bytes.extend_from_slice(&available[..bytes_to_consume]);
        total += bytes_to_consume;

        reader.consume(bytes_to_consume);

        if newline_pos.is_some() {
            // Found newline, done
            break;
        }
    }

    let line = std::str::from_utf8(&bytes).context("Invalid UTF-8 in request")?;
    buf.push_str(line);
    Ok(total)
}

/// Handle a single client connection.
async fn handle_connection(
    stream: UnixStream,
    sessions: Arc<SessionManager>,
    shutdown: Arc<Notify>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();

        // Read line with size limit to prevent memory DoS
        let bytes_read = read_line_bounded(&mut reader, &mut line, MAX_REQUEST_SIZE).await?;

        if bytes_read == 0 {
            debug!("Client disconnected");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        debug!("Received: {} bytes", trimmed.len());

        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(request) => handle_request(request, sessions.clone(), shutdown.clone()).await,
            Err(e) => Response::error(
                "unknown",
                ApiError::invalid_input_with_suggestion(
                    format!("Invalid JSON request: {}", e),
                    "Ensure the request is valid JSON with 'id' and 'command' fields. Example: {\"id\":\"1\",\"command\":{\"action\":\"list_sessions\"}}",
                ),
            ),
        };

        let response_json =
            serde_json::to_string(&response).context("Failed to serialize response")?;
        debug!("Sending: {}", response_json);

        writer
            .write_all(response_json.as_bytes())
            .await
            .context("Failed to write response")?;
        writer
            .write_all(b"\n")
            .await
            .context("Failed to write newline")?;
        writer.flush().await.context("Failed to flush")?;
    }

    Ok(())
}

/// Handle a single request and return a response.
async fn handle_request(
    request: Request,
    sessions: Arc<SessionManager>,
    shutdown: Arc<Notify>,
) -> Response {
    debug!("Handling command: {:?}", request.command);

    match request.command {
        Command::Spawn {
            command,
            session_name,
        } => handle_spawn(&request.id, &sessions, command, session_name).await,

        Command::Snapshot { session, format } => {
            handle_snapshot(&request.id, &sessions, session, format).await
        }

        Command::ListSessions => handle_list_sessions(&request.id, &sessions).await,

        Command::Kill { session } => handle_kill(&request.id, &sessions, session).await,

        Command::Type { text, session } => handle_type(&request.id, &sessions, text, session).await,

        Command::Key { key, session } => handle_key(&request.id, &sessions, key, session).await,

        Command::Click { ref_id, session } => {
            handle_click(&request.id, &sessions, ref_id, session).await
        }

        Command::Scroll {
            direction,
            amount,
            session,
        } => handle_scroll(&request.id, &sessions, direction, amount, session).await,

        Command::WaitFor {
            pattern,
            timeout_ms,
            regex,
            session,
        } => handle_wait_for(&request.id, &sessions, pattern, timeout_ms, regex, session).await,

        Command::Resize {
            cols,
            rows,
            session,
        } => handle_resize(&request.id, &sessions, cols, rows, session).await,

        Command::Shutdown => handle_shutdown(&request.id, sessions, shutdown).await,
    }
}

/// Handle spawn command.
async fn handle_spawn(
    request_id: &str,
    sessions: &SessionManager,
    command: Vec<String>,
    session_name: Option<String>,
) -> Response {
    if command.is_empty() {
        return Response::error(
            request_id,
            ApiError::invalid_input_with_suggestion(
                "No command specified",
                "Provide a command to run, e.g., 'pilotty spawn bash' or 'pilotty spawn vim file.txt'",
            ),
        );
    }

    match sessions
        .create_session(command.clone(), session_name, None)
        .await
    {
        Ok(id) => {
            info!("Created session: {}", id);
            Response::success(
                request_id,
                ResponseData::SessionCreated {
                    session_id: id.to_string(),
                    message: "Session created successfully".to_string(),
                },
            )
        }
        Err(e) => Response::error(request_id, e),
    }
}

/// Handle snapshot command.
async fn handle_snapshot(
    request_id: &str,
    sessions: &SessionManager,
    session: Option<String>,
    format: Option<SnapshotFormat>,
) -> Response {
    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Get snapshot data (drains PTY output first)
    let snapshot = match sessions.get_snapshot_data(&session_id).await {
        Ok(data) => data,
        Err(e) => return Response::error(request_id, e),
    };
    let (cursor_row, cursor_col) = snapshot.cursor_pos;

    let format = format.unwrap_or(SnapshotFormat::Full);

    match format {
        SnapshotFormat::Text => {
            // Format as plain text with cursor indicator
            let output =
                format_text_snapshot(&snapshot.text, cursor_row, cursor_col, snapshot.size);
            Response::success(
                request_id,
                ResponseData::Snapshot {
                    format: SnapshotFormat::Text,
                    content: output,
                },
            )
        }
        SnapshotFormat::Full | SnapshotFormat::Compact => {
            // Run region detection
            let mut regions = detect_regions(snapshot.cells);

            // Assign stable refs using session's RefTracker
            if let Err(e) = sessions.assign_refs(&session_id, &mut regions).await {
                return Response::error(request_id, e);
            }

            // Build full ScreenState JSON
            let snapshot_id = sessions.next_snapshot_id();
            let screen_state = ScreenState {
                snapshot_id,
                size: TerminalSize {
                    cols: snapshot.size.cols,
                    rows: snapshot.size.rows,
                },
                cursor: CursorState {
                    row: cursor_row,
                    col: cursor_col,
                    visible: snapshot.cursor_visible,
                },
                regions,
                active_region: None,
                text: if format == SnapshotFormat::Full {
                    Some(snapshot.text)
                } else {
                    None
                },
            };
            Response::success(request_id, ResponseData::ScreenState(screen_state))
        }
    }
}

/// Run all region detection algorithms on the screen.
fn detect_regions(
    cells: Vec<Vec<pilotty_core::region::Cell>>,
) -> Vec<pilotty_core::snapshot::Region> {
    use pilotty_core::region::{
        deduplicate_regions, detect_boxes, detect_highlighted_regions, detect_patterns,
        detect_underlined_shortcuts, Screen,
    };

    let screen = Screen::from_cells(cells);
    let mut ref_counter = 0;
    let mut regions = Vec::new();

    // Detect bordered boxes (dialogs, panels)
    regions.extend(detect_boxes(&screen, &mut ref_counter));

    // Detect highlighted regions (menu bars, selected items)
    regions.extend(detect_highlighted_regions(&screen, &mut ref_counter));

    // Detect patterns (buttons, checkboxes, radio buttons, menu shortcuts)
    regions.extend(detect_patterns(&screen, &mut ref_counter));

    // Detect underlined shortcuts
    regions.extend(detect_underlined_shortcuts(&screen, &mut ref_counter));

    // Deduplicate overlapping regions (e.g., button detected by both box and pattern detectors)
    deduplicate_regions(regions)
}

/// Format a plain text snapshot with cursor position indicator.
fn format_text_snapshot(
    text: &str,
    cursor_row: u16,
    cursor_col: u16,
    size: crate::daemon::pty::TermSize,
) -> String {
    let mut output = String::new();

    // Header with size and cursor info
    output.push_str(&format!(
        "--- Terminal {}x{} | Cursor: ({}, {}) ---\n",
        size.cols, size.rows, cursor_row, cursor_col
    ));

    // Screen content
    for (row_idx, line) in text.lines().enumerate() {
        if row_idx == cursor_row as usize {
            // Mark cursor position in this line using char_indices to avoid Vec<char> allocation
            let col = cursor_col as usize;

            // Find byte offset of the col-th character
            let mut char_iter = line.char_indices();
            let cursor_info = char_iter.nth(col);

            if let Some((byte_offset, cursor_char)) = cursor_info {
                // Insert cursor marker using string slices (no allocation)
                let before = &line[..byte_offset];
                let after_offset = byte_offset + cursor_char.len_utf8();
                let after = &line[after_offset..];
                output.push_str(before);
                output.push('[');
                output.push(cursor_char);
                output.push(']');
                output.push_str(after);
                output.push('\n');
            } else {
                // Cursor is at or past end of line
                output.push_str(line);
                let char_count = line.chars().count();
                if col > char_count {
                    output.push_str(&" ".repeat(col - char_count));
                }
                output.push_str("[_]\n");
            }
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    // If cursor is on a row beyond the text content
    let text_rows = text.lines().count();
    if cursor_row as usize >= text_rows {
        for _ in text_rows..cursor_row as usize {
            output.push('\n');
        }
        output.push_str(&" ".repeat(cursor_col as usize));
        output.push_str("[_]\n");
    }

    output
}

/// Handle list-sessions command.
async fn handle_list_sessions(request_id: &str, sessions: &SessionManager) -> Response {
    let session_list = sessions.list_sessions().await;
    Response::success(
        request_id,
        ResponseData::Sessions {
            sessions: session_list,
        },
    )
}

/// Handle kill command.
async fn handle_kill(
    request_id: &str,
    sessions: &SessionManager,
    session: Option<String>,
) -> Response {
    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    match sessions.kill_session(&session_id).await {
        Ok(()) => {
            info!("Killed session: {}", session_id);
            Response::success(
                request_id,
                ResponseData::Ok {
                    message: format!("Session {} killed", session_id),
                },
            )
        }
        Err(e) => Response::error(request_id, e),
    }
}

/// Handle type command - send text to PTY.
async fn handle_type(
    request_id: &str,
    sessions: &SessionManager,
    text: String,
    session: Option<String>,
) -> Response {
    use pilotty_core::input::encode_text;

    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Encode the text (handles escape sequences like \n, \t)
    let bytes = encode_text(&text);

    // Write to session
    match sessions.write_to_session(&session_id, &bytes).await {
        Ok(()) => {
            debug!("Typed {} bytes to session {}", bytes.len(), session_id);
            Response::success(
                request_id,
                ResponseData::Ok {
                    message: format!("Typed {} characters", text.len()),
                },
            )
        }
        Err(e) => Response::error(request_id, e),
    }
}

/// Handle key command - send key or key combo to PTY.
async fn handle_key(
    request_id: &str,
    sessions: &SessionManager,
    key: String,
    session: Option<String>,
) -> Response {
    use pilotty_core::input::{key_to_bytes, parse_key_combo};

    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Try to parse the key
    // Note: We check for combos only if there's a `+` that's not the entire key
    // This allows sending literal `+` as a single character
    let bytes = if key.len() > 1 && key.contains('+') {
        // Key combo like Ctrl+C (but not a literal "+")
        parse_key_combo(&key)
    } else {
        // Named key like Enter, Plus, or single character (including "+")
        key_to_bytes(&key).or_else(|| {
            // Fall back to single character
            if key.len() == 1 {
                Some(key.as_bytes().to_vec())
            } else {
                None
            }
        })
    };

    match bytes {
        Some(bytes) => match sessions.write_to_session(&session_id, &bytes).await {
            Ok(()) => {
                debug!(
                    "Sent key '{}' ({} bytes) to session {}",
                    key,
                    bytes.len(),
                    session_id
                );
                Response::success(
                    request_id,
                    ResponseData::Ok {
                        message: format!("Sent key: {}", key),
                    },
                )
            }
            Err(e) => Response::error(request_id, e),
        },
        None => Response::error(
            request_id,
            ApiError::invalid_input(format!(
                "Unknown key: '{}'. Try named keys like Enter, Tab, Escape, Up, Down, F1, etc. or combos like Ctrl+C",
                key
            )),
        ),
    }
}

/// Handle click command - click a region by ref.
async fn handle_click(
    request_id: &str,
    sessions: &SessionManager,
    ref_id: String,
    session: Option<String>,
) -> Response {
    use pilotty_core::input::encode_mouse_click_combined;
    use pilotty_core::refs::{region_center, resolve_ref_or_error};
    use pilotty_core::snapshot::RegionType;

    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Get snapshot data to detect regions
    let snapshot = match sessions.get_snapshot_data(&session_id).await {
        Ok(data) => data,
        Err(e) => return Response::error(request_id, e),
    };

    // Detect regions and assign stable refs
    let mut regions = detect_regions(snapshot.cells);
    if let Err(e) = sessions.assign_refs(&session_id, &mut regions).await {
        return Response::error(request_id, e);
    }

    // Resolve the ref
    let region = match resolve_ref_or_error(&ref_id, &regions) {
        Ok(r) => r,
        Err(e) => {
            return Response::error(
                request_id,
                ApiError::ref_not_found_with_suggestion(&ref_id, &e.suggestion),
            );
        }
    };

    // Get click position (center of region)
    let (click_x, click_y) = region_center(&region.bounds);

    // Generate mouse click sequence
    let click_bytes = encode_mouse_click_combined(click_x, click_y);

    // Send the click
    if let Err(e) = sessions.write_to_session(&session_id, &click_bytes).await {
        return Response::error(request_id, e);
    }

    // For buttons, also send Enter after a brief moment (some TUIs don't support mouse)
    if region.region_type == RegionType::Button {
        // Send Enter as a fallback for TUIs without mouse support
        if let Err(e) = sessions.write_to_session(&session_id, b"\r").await {
            return Response::error(request_id, e);
        }
    }

    debug!(
        "Clicked region {} at ({}, {}) in session {}",
        ref_id, click_x, click_y, session_id
    );

    Response::success(
        request_id,
        ResponseData::Ok {
            message: format!(
                "Clicked {} ({}) at ({}, {})",
                ref_id, region.text, click_x, click_y
            ),
        },
    )
}

/// Handle scroll command.
async fn handle_scroll(
    request_id: &str,
    sessions: &SessionManager,
    direction: pilotty_core::protocol::ScrollDirection,
    amount: u32,
    session: Option<String>,
) -> Response {
    use pilotty_core::input::encode_scroll;

    if amount > MAX_SCROLL_AMOUNT {
        return Response::error(
            request_id,
            ApiError::invalid_input_with_suggestion(
                format!(
                    "Scroll amount {} exceeds maximum {}",
                    amount, MAX_SCROLL_AMOUNT
                ),
                "Use a smaller scroll amount (<= 1000).",
            ),
        );
    }

    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Get terminal size and scroll at center
    let (scroll_x, scroll_y) = match sessions.get_terminal_size(&session_id).await {
        Ok(size) => (size.cols / 2, size.rows / 2),
        Err(_) => (40, 12), // Fallback if session gone mid-request
    };

    // Send scroll events
    for _ in 0..amount {
        let scroll_bytes = encode_scroll(direction, scroll_x, scroll_y);
        if let Err(e) = sessions.write_to_session(&session_id, &scroll_bytes).await {
            return Response::error(request_id, e);
        }
    }

    let dir_str = match direction {
        pilotty_core::protocol::ScrollDirection::Up => "up",
        pilotty_core::protocol::ScrollDirection::Down => "down",
    };

    debug!(
        "Scrolled {} {} times in session {}",
        dir_str, amount, session_id
    );

    Response::success(
        request_id,
        ResponseData::Ok {
            message: format!("Scrolled {} {} times", dir_str, amount),
        },
    )
}

/// Handle resize command.
async fn handle_resize(
    request_id: &str,
    sessions: &SessionManager,
    cols: u16,
    rows: u16,
    session: Option<String>,
) -> Response {
    // Validate dimensions
    if cols == 0 || rows == 0 {
        return Response::error(
            request_id,
            ApiError::invalid_input("Terminal dimensions must be greater than 0"),
        );
    }

    // Resolve session
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Resize the session
    match sessions.resize_session(&session_id, cols, rows).await {
        Ok(()) => {
            debug!("Resized session {} to {}x{}", session_id, cols, rows);
            Response::success(
                request_id,
                ResponseData::Ok {
                    message: format!("Resized terminal to {}x{}", cols, rows),
                },
            )
        }
        Err(e) => Response::error(request_id, e),
    }
}

/// Handle wait-for command - poll for text pattern.
async fn handle_wait_for(
    request_id: &str,
    sessions: &SessionManager,
    pattern: String,
    timeout_ms: Option<u64>,
    regex: Option<bool>,
    session: Option<String>,
) -> Response {
    use std::time::{Duration, Instant};

    const POLL_INTERVAL_MS: u64 = 100;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(30000));
    let use_regex = regex.unwrap_or(false);

    // Resolve session first
    let session_id = match sessions.resolve_session(session.as_deref()).await {
        Ok(id) => id,
        Err(e) => return Response::error(request_id, e),
    };

    // Compile regex if needed
    let compiled_regex = if use_regex {
        match regex::Regex::new(&pattern) {
            Ok(r) => Some(r),
            Err(e) => {
                return Response::error(
                    request_id,
                    ApiError::invalid_input_with_suggestion(
                        format!("Invalid regex pattern: {}", e),
                        "Check your regex syntax. Common issues: unescaped special chars, unbalanced parentheses.",
                    ),
                );
            }
        }
    } else {
        None
    };

    let start = Instant::now();

    // Poll loop
    loop {
        // Check timeout first
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return Response::error(
                request_id,
                ApiError::command_failed_with_suggestion(
                    format!(
                        "Timeout waiting for '{}' after {}ms",
                        pattern,
                        elapsed.as_millis()
                    ),
                    "The pattern was not found within the timeout. Try increasing --timeout or check if the expected text actually appears.",
                ),
            );
        }

        // Get current screen text
        let snapshot = match sessions.get_snapshot_data(&session_id).await {
            Ok(data) => data,
            Err(e) => return Response::error(request_id, e),
        };

        // Check for match
        let matched = if let Some(ref re) = compiled_regex {
            re.find(&snapshot.text).map(|m| m.as_str().to_string())
        } else {
            // Plain text search
            if snapshot.text.contains(&pattern) {
                Some(pattern.clone())
            } else {
                None
            }
        };

        if let Some(matched_text) = matched {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            debug!(
                "Found '{}' after {}ms in session {}",
                matched_text, elapsed_ms, session_id
            );
            return Response::success(
                request_id,
                ResponseData::WaitForResult {
                    found: true,
                    matched_text: Some(matched_text),
                    elapsed_ms,
                },
            );
        }

        // Wait before next poll
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

/// Handle shutdown command - gracefully stop the daemon.
///
/// Kills all sessions and signals the main run loop to exit.
/// The DaemonServer's Drop impl cleans up the socket and PID files.
async fn handle_shutdown(
    request_id: &str,
    sessions: Arc<SessionManager>,
    shutdown: Arc<Notify>,
) -> Response {
    info!("Received shutdown command, stopping daemon");

    // Spawn shutdown task to run after we return the response
    tokio::spawn(async move {
        // Kill all sessions first
        kill_all_sessions(&sessions).await;

        // Brief delay to allow response to flush before signaling shutdown
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Signal main loop to exit (allows Drop to clean up files)
        shutdown.notify_waiters();
    });

    Response::success(
        request_id,
        ResponseData::Ok {
            message: "Daemon shutting down".to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pilotty_core::error::ErrorCode;
    use pilotty_core::protocol::Command;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;
    use tokio::time::timeout;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_daemon_accepts_and_responds() {
        // Use a temp socket path
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-test-{}.sock", std::process::id()));

        // Start server
        let pid_path = socket_path.with_extension("pid");
        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            // Run server for a short time
            let _ = timeout(Duration::from_secs(2), server.run()).await;
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect client
        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send a request
        let request = Request {
            id: "test-1".to_string(),
            command: Command::ListSessions,
        };
        let request_json = serde_json::to_string(&request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("Failed to write");
        writer.write_all(b"\n").await.expect("Failed to write");
        writer.flush().await.expect("Failed to flush");

        // Read response
        let mut response_line = String::new();
        let read_result = timeout(Duration::from_secs(1), reader.read_line(&mut response_line))
            .await
            .expect("Timeout reading response")
            .expect("Failed to read");

        assert!(read_result > 0, "Should have received a response");

        let response: Response =
            serde_json::from_str(&response_line).expect("Failed to parse response");
        assert!(response.success);
        assert_eq!(response.id, "test-1");

        // Clean up
        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_read_line_bounded_handles_utf8_chunks() {
        let data = "hello 你好\n".as_bytes().to_vec();
        let cursor = std::io::Cursor::new(data);
        let mut reader = BufReader::with_capacity(1, cursor);
        let mut buf = String::new();

        let bytes = read_line_bounded(&mut reader, &mut buf, 1024)
            .await
            .expect("read line");

        assert!(bytes > 0);
        assert_eq!(buf, "hello 你好\n");
    }

    #[tokio::test]
    async fn test_bind_to_creates_socket_parent_dir() {
        let short_id = Uuid::new_v4().simple().to_string();
        let base_dir =
            std::path::PathBuf::from("/tmp").join(format!("pilotty-custom-{}", &short_id[..8]));
        let socket_dir = base_dir.join("nested");
        let socket_path = socket_dir.join("pilotty.sock");
        let pid_path = socket_path.with_extension("pid");

        let _ = std::fs::remove_dir_all(&base_dir);

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        assert!(socket_dir.exists());

        drop(server);
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&pid_path);
        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[tokio::test]
    async fn test_scroll_rejects_large_amount() {
        let short_id = Uuid::new_v4().simple().to_string();
        let socket_path = std::path::PathBuf::from("/tmp")
            .join(format!("pilotty-scroll-{}.sock", &short_id[..8]));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(2), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let request = Request {
            id: "scroll-1".to_string(),
            command: Command::Scroll {
                direction: pilotty_core::protocol::ScrollDirection::Down,
                amount: MAX_SCROLL_AMOUNT + 1,
                session: None,
            },
        };
        let request_json = serde_json::to_string(&request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let response: Response = serde_json::from_str(&response_line).expect("parse response");
        assert!(!response.success);
        let error = response.error.expect("error response");
        assert_eq!(error.code, ErrorCode::InvalidInput);

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&pid_path);
    }

    #[tokio::test]
    async fn test_spawn_and_snapshot() {
        // Use a temp socket path
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-snap-{}.sock", std::process::id()));

        // Start server
        let pid_path = socket_path.with_extension("pid");
        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect client
        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session running echo
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["echo".to_string(), "hello from test".to_string()],
                session_name: Some("test-snap".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        // Read spawn response
        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let spawn_response: Response =
            serde_json::from_str(&response_line).expect("parse spawn response");
        assert!(spawn_response.success, "Spawn should succeed");

        // Give the echo command time to produce output
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Take a snapshot
        let snap_request = Request {
            id: "snap-1".to_string(),
            command: Command::Snapshot {
                session: Some("test-snap".to_string()),
                format: Some(SnapshotFormat::Text),
            },
        };
        let snap_json = serde_json::to_string(&snap_request).unwrap();
        writer.write_all(snap_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        // Read snapshot response
        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let snap_response: Response =
            serde_json::from_str(&response_line).expect("parse snap response");
        assert!(snap_response.success, "Snapshot should succeed");

        // Verify the snapshot contains our text
        if let Some(ResponseData::Snapshot { format, content }) = snap_response.data {
            assert_eq!(format, SnapshotFormat::Text);
            assert!(
                content.contains("hello from test"),
                "Snapshot should contain 'hello from test', got:\n{}",
                content
            );
            // Verify header format
            assert!(
                content.contains("Terminal 80x24"),
                "Snapshot should have terminal size header"
            );
            assert!(
                content.contains("Cursor:"),
                "Snapshot should show cursor position"
            );
        } else {
            panic!("Expected Snapshot response data");
        }

        // Clean up
        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_snapshot_full_format() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-full-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["echo".to_string(), "full format test".to_string()],
                session_name: Some("full-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Request snapshot with Full format
        let snap_request = Request {
            id: "snap-full".to_string(),
            command: Command::Snapshot {
                session: Some("full-test".to_string()),
                format: Some(SnapshotFormat::Full),
            },
        };
        let snap_json = serde_json::to_string(&snap_request).unwrap();
        writer.write_all(snap_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let snap_response: Response =
            serde_json::from_str(&response_line).expect("parse snap response");
        assert!(snap_response.success, "Snapshot should succeed");

        // Verify ScreenState structure
        if let Some(ResponseData::ScreenState(screen_state)) = snap_response.data {
            // Check snapshot_id is non-zero
            assert!(
                screen_state.snapshot_id > 0,
                "snapshot_id should be positive"
            );

            // Check size
            assert_eq!(screen_state.size.cols, 80, "Default cols should be 80");
            assert_eq!(screen_state.size.rows, 24, "Default rows should be 24");

            // Check cursor position is valid
            assert!(
                screen_state.cursor.row < screen_state.size.rows,
                "Cursor row should be within bounds"
            );
            assert!(
                screen_state.cursor.col < screen_state.size.cols,
                "Cursor col should be within bounds"
            );

            // Check text is included in Full format
            assert!(
                screen_state.text.is_some(),
                "Full format should include text"
            );
            let text = screen_state.text.unwrap();
            assert!(
                text.contains("full format test"),
                "Text should contain output: {}",
                text
            );
        } else {
            panic!(
                "Expected ScreenState response data, got: {:?}",
                snap_response.data
            );
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_snapshot_detects_regions() {
        use pilotty_core::snapshot::RegionType;

        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-regions-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session that outputs UI-like patterns
        // Using printf to output patterns that region detection should find
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec![
                    "printf".to_string(),
                    "[ OK ]  [ Cancel ]\\n[x] Option A\\n[ ] Option B\\n".to_string(),
                ],
                session_name: Some("region-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        // Give time for output
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Request snapshot with Full format
        let snap_request = Request {
            id: "snap-regions".to_string(),
            command: Command::Snapshot {
                session: Some("region-test".to_string()),
                format: Some(SnapshotFormat::Full),
            },
        };
        let snap_json = serde_json::to_string(&snap_request).unwrap();
        writer.write_all(snap_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let snap_response: Response =
            serde_json::from_str(&response_line).expect("parse snap response");
        assert!(snap_response.success, "Snapshot should succeed");

        // Verify regions are detected
        if let Some(ResponseData::ScreenState(screen_state)) = snap_response.data {
            // Should detect at least some regions
            assert!(
                !screen_state.regions.is_empty(),
                "Should detect some regions, text was:\n{}",
                screen_state.text.as_deref().unwrap_or("(no text)")
            );

            // Check for buttons
            let buttons: Vec<_> = screen_state
                .regions
                .iter()
                .filter(|r| r.region_type == RegionType::Button)
                .collect();

            // Check for checkboxes
            let checkboxes: Vec<_> = screen_state
                .regions
                .iter()
                .filter(|r| r.region_type == RegionType::Checkbox)
                .collect();

            // We expect at least 2 buttons (OK, Cancel) and 2 checkboxes
            assert!(
                buttons.len() >= 2,
                "Should detect at least 2 buttons, found {}: {:?}",
                buttons.len(),
                buttons
            );
            assert!(
                checkboxes.len() >= 2,
                "Should detect at least 2 checkboxes, found {}: {:?}",
                checkboxes.len(),
                checkboxes
            );

            // Verify ref IDs are assigned
            for region in &screen_state.regions {
                assert!(
                    region.ref_id.as_str().starts_with("@e"),
                    "Ref ID should start with @e: {}",
                    region.ref_id
                );
            }
        } else {
            panic!(
                "Expected ScreenState response data, got: {:?}",
                snap_response.data
            );
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_type_command() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-type-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a cat session (echoes input)
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["cat".to_string()],
                session_name: Some("type-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let spawn_response: Response =
            serde_json::from_str(&response_line).expect("parse spawn response");
        assert!(spawn_response.success, "Spawn should succeed");

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Type some text
        let type_request = Request {
            id: "type-1".to_string(),
            command: Command::Type {
                text: "Hello World".to_string(),
                session: Some("type-test".to_string()),
            },
        };
        let type_json = serde_json::to_string(&type_request).unwrap();
        writer.write_all(type_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let type_response: Response =
            serde_json::from_str(&response_line).expect("parse type response");
        assert!(type_response.success, "Type should succeed");

        // Give time for cat to echo
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Take a snapshot and verify the text appears
        let snap_request = Request {
            id: "snap-1".to_string(),
            command: Command::Snapshot {
                session: Some("type-test".to_string()),
                format: Some(SnapshotFormat::Text),
            },
        };
        let snap_json = serde_json::to_string(&snap_request).unwrap();
        writer.write_all(snap_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let snap_response: Response =
            serde_json::from_str(&response_line).expect("parse snap response");
        assert!(snap_response.success, "Snapshot should succeed");

        // Verify the typed text appears in the snapshot
        if let Some(ResponseData::Snapshot { content, .. }) = snap_response.data {
            assert!(
                content.contains("Hello World"),
                "Snapshot should contain typed text 'Hello World', got:\n{}",
                content
            );
        } else {
            panic!("Expected Snapshot response data");
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_key_command() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-key-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a cat session
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["cat".to_string()],
                session_name: Some("key-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send a key
        let key_request = Request {
            id: "key-1".to_string(),
            command: Command::Key {
                key: "Enter".to_string(),
                session: Some("key-test".to_string()),
            },
        };
        let key_json = serde_json::to_string(&key_request).unwrap();
        writer.write_all(key_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let key_response: Response =
            serde_json::from_str(&response_line).expect("parse key response");
        assert!(key_response.success, "Key should succeed");

        // Test Ctrl+C (which will terminate cat)
        let ctrlc_request = Request {
            id: "key-2".to_string(),
            command: Command::Key {
                key: "Ctrl+C".to_string(),
                session: Some("key-test".to_string()),
            },
        };
        let ctrlc_json = serde_json::to_string(&ctrlc_request).unwrap();
        writer
            .write_all(ctrlc_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let ctrlc_response: Response =
            serde_json::from_str(&response_line).expect("parse ctrl+c response");
        assert!(ctrlc_response.success, "Ctrl+C should succeed");

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_click_command() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-click-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session that outputs UI-like patterns
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["printf".to_string(), "[ OK ]  [ Cancel ]\\n".to_string()],
                session_name: Some("click-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Try to click a region that exists
        // First get the snapshot to see what refs are available
        let snap_request = Request {
            id: "snap-1".to_string(),
            command: Command::Snapshot {
                session: Some("click-test".to_string()),
                format: Some(SnapshotFormat::Full),
            },
        };
        let snap_json = serde_json::to_string(&snap_request).unwrap();
        writer.write_all(snap_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let snap_response: Response =
            serde_json::from_str(&response_line).expect("parse snap response");
        assert!(snap_response.success, "Snapshot should succeed");

        // Get the first region ref
        let first_ref = if let Some(ResponseData::ScreenState(screen_state)) = &snap_response.data {
            if !screen_state.regions.is_empty() {
                screen_state.regions[0].ref_id.to_string()
            } else {
                "@e1".to_string() // fallback, will fail gracefully
            }
        } else {
            "@e1".to_string()
        };

        // Now click the region
        let click_request = Request {
            id: "click-1".to_string(),
            command: Command::Click {
                ref_id: first_ref.clone(),
                session: Some("click-test".to_string()),
            },
        };
        let click_json = serde_json::to_string(&click_request).unwrap();
        writer
            .write_all(click_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let click_response: Response =
            serde_json::from_str(&response_line).expect("parse click response");
        assert!(
            click_response.success,
            "Click should succeed, got: {:?}",
            click_response
        );

        // Test clicking an invalid ref
        let bad_click_request = Request {
            id: "click-2".to_string(),
            command: Command::Click {
                ref_id: "@e999".to_string(),
                session: Some("click-test".to_string()),
            },
        };
        let bad_click_json = serde_json::to_string(&bad_click_request).unwrap();
        writer
            .write_all(bad_click_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let bad_click_response: Response =
            serde_json::from_str(&response_line).expect("parse bad click response");
        assert!(
            !bad_click_response.success,
            "Click on invalid ref should fail"
        );
        // Verify error has suggestion
        if let Some(err) = &bad_click_response.error {
            assert!(err.suggestion.is_some(), "Error should have a suggestion");
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_scroll_command() {
        use pilotty_core::protocol::ScrollDirection;

        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-scroll-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["cat".to_string()],
                session_name: Some("scroll-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Scroll up
        let scroll_up_request = Request {
            id: "scroll-1".to_string(),
            command: Command::Scroll {
                direction: ScrollDirection::Up,
                amount: 3,
                session: Some("scroll-test".to_string()),
            },
        };
        let scroll_json = serde_json::to_string(&scroll_up_request).unwrap();
        writer
            .write_all(scroll_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let scroll_response: Response =
            serde_json::from_str(&response_line).expect("parse scroll response");
        assert!(scroll_response.success, "Scroll up should succeed");

        // Scroll down
        let scroll_down_request = Request {
            id: "scroll-2".to_string(),
            command: Command::Scroll {
                direction: ScrollDirection::Down,
                amount: 5,
                session: Some("scroll-test".to_string()),
            },
        };
        let scroll_json = serde_json::to_string(&scroll_down_request).unwrap();
        writer
            .write_all(scroll_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let scroll_response: Response =
            serde_json::from_str(&response_line).expect("parse scroll response");
        assert!(scroll_response.success, "Scroll down should succeed");

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_wait_for_plain_text() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-waitfor-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(10), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session that outputs text
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["echo".to_string(), "hello world marker".to_string()],
                session_name: Some("waitfor-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        // Give echo time to run
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Wait for the text
        let wait_request = Request {
            id: "wait-1".to_string(),
            command: Command::WaitFor {
                pattern: "marker".to_string(),
                timeout_ms: Some(5000),
                regex: Some(false),
                session: Some("waitfor-test".to_string()),
            },
        };
        let wait_json = serde_json::to_string(&wait_request).unwrap();
        writer.write_all(wait_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(6), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let wait_response: Response =
            serde_json::from_str(&response_line).expect("parse wait response");
        assert!(wait_response.success, "Wait-for should succeed");

        // Verify response structure
        if let Some(ResponseData::WaitForResult {
            found,
            matched_text,
            elapsed_ms,
        }) = wait_response.data
        {
            assert!(found, "Should have found the pattern");
            assert_eq!(matched_text, Some("marker".to_string()));
            assert!(elapsed_ms < 5000, "Should find quickly, not near timeout");
        } else {
            panic!("Expected WaitForResult, got: {:?}", wait_response.data);
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_wait_for_regex() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-waitfor-re-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(10), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["echo".to_string(), "version 1.2.3 ready".to_string()],
                session_name: Some("waitfor-re-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Wait for regex pattern (version number)
        let wait_request = Request {
            id: "wait-re".to_string(),
            command: Command::WaitFor {
                pattern: r"version \d+\.\d+\.\d+".to_string(),
                timeout_ms: Some(5000),
                regex: Some(true),
                session: Some("waitfor-re-test".to_string()),
            },
        };
        let wait_json = serde_json::to_string(&wait_request).unwrap();
        writer.write_all(wait_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(6), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let wait_response: Response =
            serde_json::from_str(&response_line).expect("parse wait response");
        assert!(wait_response.success, "Wait-for regex should succeed");

        if let Some(ResponseData::WaitForResult {
            found,
            matched_text,
            ..
        }) = wait_response.data
        {
            assert!(found, "Should have found the pattern");
            assert_eq!(matched_text, Some("version 1.2.3".to_string()));
        } else {
            panic!("Expected WaitForResult, got: {:?}", wait_response.data);
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_wait_for_timeout() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-waitfor-to-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(10), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a long-running session (cat waits for input)
        // We need a process that stays alive so the session isn't cleaned up
        // before the wait-for timeout occurs.
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["cat".to_string()],
                session_name: Some("waitfor-to-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Wait for text that won't appear (short timeout)
        let wait_request = Request {
            id: "wait-to".to_string(),
            command: Command::WaitFor {
                pattern: "nonexistent pattern xyz".to_string(),
                timeout_ms: Some(500), // Short timeout
                regex: Some(false),
                session: Some("waitfor-to-test".to_string()),
            },
        };
        let wait_json = serde_json::to_string(&wait_request).unwrap();
        writer.write_all(wait_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(3), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let wait_response: Response =
            serde_json::from_str(&response_line).expect("parse wait response");
        assert!(!wait_response.success, "Wait-for should fail on timeout");

        // Verify error has suggestion
        if let Some(err) = &wait_response.error {
            assert!(
                err.suggestion.is_some(),
                "Timeout error should have suggestion"
            );
            assert!(
                err.message.contains("Timeout"),
                "Error should mention timeout"
            );
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_wait_for_invalid_regex() {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("pilotty-waitfor-bad-{}.sock", std::process::id()));
        let pid_path = socket_path.with_extension("pid");

        let server = DaemonServer::bind_to(socket_path.clone(), pid_path.clone())
            .await
            .expect("Failed to bind server");

        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(5), server.run()).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("Failed to connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Spawn a session
        let spawn_request = Request {
            id: "spawn-1".to_string(),
            command: Command::Spawn {
                command: vec!["echo".to_string(), "test".to_string()],
                session_name: Some("waitfor-bad-test".to_string()),
            },
        };
        let request_json = serde_json::to_string(&spawn_request).unwrap();
        writer
            .write_all(request_json.as_bytes())
            .await
            .expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        let mut response_line = String::new();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        // Try invalid regex
        let wait_request = Request {
            id: "wait-bad".to_string(),
            command: Command::WaitFor {
                pattern: "[invalid(regex".to_string(), // Unbalanced brackets
                timeout_ms: Some(1000),
                regex: Some(true),
                session: Some("waitfor-bad-test".to_string()),
            },
        };
        let wait_json = serde_json::to_string(&wait_request).unwrap();
        writer.write_all(wait_json.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");

        response_line.clear();
        timeout(Duration::from_secs(2), reader.read_line(&mut response_line))
            .await
            .expect("timeout")
            .expect("read");

        let wait_response: Response =
            serde_json::from_str(&response_line).expect("parse wait response");
        assert!(
            !wait_response.success,
            "Wait-for with invalid regex should fail"
        );

        if let Some(err) = &wait_response.error {
            assert!(
                err.message.contains("Invalid regex"),
                "Error should mention invalid regex"
            );
            assert!(
                err.suggestion.is_some(),
                "Should have suggestion for regex fix"
            );
        }

        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }
}
