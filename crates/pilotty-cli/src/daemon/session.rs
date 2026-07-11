//! Session manager for tracking active PTY sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use portable_pty::ExitStatus;
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info};

use pilotty_core::elements::classify::{detect, ClassifyContext};
use pilotty_core::elements::Element;
use pilotty_core::error::ApiError;
use pilotty_core::protocol::SessionInfo;
use pilotty_core::snapshot::{compute_content_hash, CursorState, ScreenState, TerminalSize};

use crate::daemon::pty::{AsyncPtyHandle, PtySession, TermSize};
use crate::daemon::retention::{RetentionRing, RetentionSnapshot, DEFAULT_RETAIN_BYTES};
use crate::daemon::terminal::TerminalEmulator;
use crate::daemon::tombstone::{
    ExitMetadata, Tombstone, TombstoneStore, TOMBSTONE_CAPACITY, TOMBSTONE_OUTPUT_BYTES,
    TOMBSTONE_TTL,
};

/// Unique identifier for a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl SessionId {
    /// Generate a new unique session ID.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Atomically observed screen data for a session.
pub(crate) struct SnapshotData {
    pub(crate) text: String,
    pub(crate) cursor_pos: (u16, u16),
    pub(crate) cursor_visible: bool,
    pub(crate) size: TermSize,
    /// Detected UI elements (computed on demand).
    pub(crate) elements: Option<Vec<Element>>,
    /// Hash of screen content for change detection.
    pub(crate) content_hash: u64,
    /// Revision matching this exact screen state.
    pub(crate) revision: u64,
}

/// Latest state published by a session's output pump.
#[derive(Debug, Clone, Copy)]
struct PumpState {
    revision: u64,
    last_output_at: Instant,
    output_closed: bool,
}

/// Terminal state whose content and revision are captured atomically.
struct ObservedTerminal {
    emulator: TerminalEmulator,
    revision: u64,
    size: TermSize,
}

/// Owned pump task that cannot detach when its session is dropped.
struct PumpTask {
    handle: Option<JoinHandle<()>>,
}

impl PumpTask {
    fn new(handle: JoinHandle<()>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    async fn abort_and_wait(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        handle.abort();
        if let Err(error) = handle.await {
            if !error.is_cancelled() {
                debug!("Output pump failed during shutdown: {}", error);
            }
        }
    }

    async fn wait(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        if let Err(error) = handle.await {
            debug!("Output pump failed: {}", error);
        }
    }
}

impl Drop for PumpTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// An active PTY session.
struct Session {
    /// Unique session ID.
    id: SessionId,
    /// Optional human-readable name.
    name: Option<String>,
    /// Command that was spawned.
    command: Vec<String>,
    cwd: Option<String>,
    /// When the session was created.
    created_at: DateTime<Utc>,
    /// Async handle for PTY I/O.
    pty: AsyncPtyHandle,
    retention: Arc<Mutex<RetentionRing>>,
    observed_terminal: Arc<Mutex<ObservedTerminal>>,
    pump_state: watch::Receiver<PumpState>,
    pump_task: Mutex<PumpTask>,
    process_exit: std::sync::Mutex<Option<ProcessExit>>,
}

#[derive(Clone)]
struct ProcessExit {
    status: ExitStatus,
    observed_at: Instant,
}

impl Session {
    /// Get session info for protocol responses.
    fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.0.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            created_at: self.created_at.to_rfc3339(),
        }
    }

    /// Check if terminal is in application cursor mode.
    async fn application_cursor(&self) -> bool {
        self.observed_terminal
            .lock()
            .await
            .emulator
            .application_cursor()
    }

    /// Write bytes to the PTY (send input to the terminal).
    async fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        self.pty.write(data).await
    }

    async fn snapshot(&self, with_elements: bool) -> SnapshotData {
        let terminal = self.observed_terminal.lock().await;
        let text = terminal.emulator.get_text();
        let cursor_pos = terminal.emulator.cursor_position();
        let cursor_visible = terminal.emulator.cursor_visible();
        let elements = if with_elements {
            let (cursor_row, cursor_col) = cursor_pos;
            let context = ClassifyContext::new().with_cursor(cursor_row, cursor_col);
            Some(detect(&terminal.emulator, &context))
        } else {
            None
        };

        SnapshotData {
            content_hash: compute_content_hash(&text),
            text,
            cursor_pos,
            cursor_visible,
            size: terminal.size,
            elements,
            revision: terminal.revision,
        }
    }

    fn pump_state(&self) -> PumpState {
        *self.pump_state.borrow()
    }

    fn observe_process_exit(&self) -> anyhow::Result<Option<ProcessExit>> {
        let mut process_exit = self
            .process_exit
            .lock()
            .map_err(|_| anyhow::anyhow!("Process exit mutex poisoned"))?;
        if let Some(exit) = process_exit.as_ref() {
            return Ok(Some(exit.clone()));
        }

        let Some(status) = self.pty.exit_status()? else {
            return Ok(None);
        };
        let exit = ProcessExit {
            status,
            observed_at: Instant::now(),
        };
        *process_exit = Some(exit.clone());
        Ok(Some(exit))
    }

    async fn finish_pump(&self, output_complete: bool) {
        let mut pump_task = self.pump_task.lock().await;
        if output_complete {
            pump_task.wait().await;
        } else {
            pump_task.abort_and_wait().await;
        }
    }

    async fn wait_for_output_close(&self) -> bool {
        let mut pump_state = self.pump_state.clone();
        if pump_state.borrow().output_closed {
            return true;
        }

        tokio::time::timeout(EXIT_DRAIN_TIMEOUT, async move {
            loop {
                if pump_state.changed().await.is_err() {
                    return pump_state.borrow().output_closed;
                }
                if pump_state.borrow_and_update().output_closed {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false)
    }

    async fn shutdown(&self) -> bool {
        self.pty.terminate();
        let output_complete = self.wait_for_output_close().await;
        self.finish_pump(output_complete).await;
        let _exit = self.observe_process_exit();
        output_complete
    }

    async fn final_tombstone(
        &self,
        snapshot_id: u64,
        output_complete: bool,
        killed_by_client: bool,
    ) -> Tombstone {
        let snapshot = self.snapshot(true).await;
        let output = self
            .retention
            .lock()
            .await
            .snapshot()
            .into_tail(TOMBSTONE_OUTPUT_BYTES);
        let process_exit = self
            .process_exit
            .lock()
            .ok()
            .and_then(|exit| exit.as_ref().cloned());
        let exit = process_exit
            .as_ref()
            .map(|exit| ExitMetadata {
                code: Some(exit.status.exit_code()),
                signal: exit.status.signal().map(ToOwned::to_owned),
                success: exit.status.success(),
                killed_by_client,
            })
            .unwrap_or(ExitMetadata {
                code: None,
                signal: None,
                success: false,
                killed_by_client,
            });
        let ended_at_monotonic = Instant::now();
        Tombstone {
            id: self.id.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            created_at: self.created_at,
            ended_at: Utc::now(),
            ended_at_monotonic,
            exit,
            output_complete,
            final_screen: ScreenState {
                snapshot_id,
                size: TerminalSize {
                    cols: snapshot.size.cols,
                    rows: snapshot.size.rows,
                },
                cursor: CursorState {
                    row: snapshot.cursor_pos.0,
                    col: snapshot.cursor_pos.1,
                    visible: snapshot.cursor_visible,
                },
                text: Some(snapshot.text),
                elements: snapshot.elements,
                content_hash: Some(snapshot.content_hash),
            },
            output,
        }
    }
}

/// Result of waiting for a session observation without exposing watch semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservationEvent {
    Updated,
    OutputClosed,
    Deadline,
    PumpFailed,
}

/// Subscription to one session's atomically observed terminal state.
pub(crate) struct SessionObserver {
    session: Arc<Session>,
    pump_state: watch::Receiver<PumpState>,
}

#[derive(Clone)]
pub(crate) enum SessionEvidence {
    Live(SessionId),
    Exited(Box<Tombstone>),
}

impl SessionObserver {
    /// Capture the current screen after marking the current pump state as observed.
    /// Output arriving after that mark remains visible to `wait_for_update`.
    pub(crate) async fn current(&mut self, with_elements: bool) -> SnapshotData {
        {
            let _state = self.pump_state.borrow_and_update();
        }
        self.session.snapshot(with_elements).await
    }

    /// Wait for output, EOF, pump failure, or the supplied duration.
    pub(crate) async fn wait_for_update(&mut self, duration: Duration) -> ObservationEvent {
        match tokio::time::timeout(duration, self.pump_state.changed()).await {
            Err(_) => ObservationEvent::Deadline,
            Ok(Ok(())) => {
                if self.pump_state.borrow().output_closed {
                    ObservationEvent::OutputClosed
                } else {
                    ObservationEvent::Updated
                }
            }
            Ok(Err(_)) => {
                if self.pump_state.borrow().output_closed {
                    ObservationEvent::OutputClosed
                } else {
                    ObservationEvent::PumpFailed
                }
            }
        }
    }
}

const MAX_PUMP_BATCH_BYTES: usize = 1024 * 1024;
const EXIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

async fn run_output_pump(
    mut read_rx: mpsc::Receiver<Vec<u8>>,
    retention: Arc<Mutex<RetentionRing>>,
    observed_terminal: Arc<Mutex<ObservedTerminal>>,
    state_tx: watch::Sender<PumpState>,
) {
    let mut pending = None;
    let mut last_output_at = Instant::now();

    loop {
        let first = match pending.take() {
            Some(data) => data,
            None => match read_rx.recv().await {
                Some(data) => data,
                None => {
                    let revision = observed_terminal.lock().await.revision;
                    state_tx.send_replace(PumpState {
                        revision,
                        last_output_at,
                        output_closed: true,
                    });
                    return;
                }
            },
        };

        let mut batch = first;
        while batch.len() < MAX_PUMP_BATCH_BYTES {
            match read_rx.try_recv() {
                Ok(data) if batch.len() + data.len() <= MAX_PUMP_BATCH_BYTES => {
                    batch.extend_from_slice(&data);
                }
                Ok(data) => {
                    pending = Some(data);
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        retention.lock().await.append(&batch);
        let revision = {
            let mut terminal = observed_terminal.lock().await;
            terminal.emulator.feed(&batch);
            terminal.revision = terminal.revision.saturating_add(1);
            terminal.revision
        };
        last_output_at = Instant::now();
        state_tx.send_replace(PumpState {
            revision,
            last_output_at,
            output_closed: false,
        });
    }
}

/// Maximum number of concurrent sessions to prevent resource exhaustion.
const MAX_SESSIONS: usize = 100;

/// Manages active PTY sessions.
///
/// Thread-safe via interior mutability with RwLock.
pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
    tombstones: Mutex<TombstoneStore>,
    default_retain_bytes: usize,
    /// Global snapshot counter for unique snapshot IDs.
    snapshot_counter: AtomicU64,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self::with_default_retain_bytes(DEFAULT_RETAIN_BYTES)
    }

    /// Create a session manager with an explicit default retention limit.
    pub(crate) fn with_default_retain_bytes(default_retain_bytes: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            tombstones: Mutex::new(TombstoneStore::new(TOMBSTONE_CAPACITY, TOMBSTONE_TTL)),
            default_retain_bytes,
            snapshot_counter: AtomicU64::new(1),
        }
    }

    /// Get the next snapshot ID (incrementing counter).
    ///
    /// Uses Relaxed ordering since snapshot IDs only need uniqueness, not global ordering.
    pub fn next_snapshot_id(&self) -> u64 {
        self.snapshot_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Create a new session by spawning a PTY.
    ///
    /// Returns the session ID on success.
    /// Returns an error if:
    /// - Maximum session limit is reached
    /// - A session with the same name already exists
    /// - Spawn fails
    ///
    /// If `cwd` is provided, the spawned process runs in that directory.
    #[cfg(test)]
    pub async fn create_session(
        &self,
        command: Vec<String>,
        name: Option<String>,
        size: Option<TermSize>,
        cwd: Option<String>,
    ) -> Result<SessionId, ApiError> {
        self.create_session_with_retention(command, name, size, cwd, None)
            .await
    }

    pub async fn create_session_with_retention(
        &self,
        command: Vec<String>,
        name: Option<String>,
        size: Option<TermSize>,
        cwd: Option<String>,
        retain_bytes: Option<usize>,
    ) -> Result<SessionId, ApiError> {
        let name = name.or_else(|| Some("default".to_string()));

        // Check session limit and name uniqueness before spawning to prevent
        // expensive work when the request will fail.
        {
            let sessions = self.sessions.read().await;
            if sessions.len() >= MAX_SESSIONS {
                return Err(ApiError::session_limit_reached(MAX_SESSIONS));
            }
            if let Some(ref n) = name {
                if sessions.values().any(|s| s.name.as_deref() == Some(n)) {
                    return Err(ApiError::duplicate_session_name(n));
                }
            }
        }

        let size = size.unwrap_or_default();

        // Spawn the PTY session
        let pty_session = PtySession::spawn(&command, size, cwd.as_deref())
            .map_err(|error| ApiError::spawn_failed(&command, &format!("{error:#}")))?;

        // Wrap in async handle and transfer sole output ownership to the pump.
        let (pty, read_rx) = AsyncPtyHandle::new(pty_session)
            .map_err(|error| ApiError::spawn_failed(&command, &format!("{error:#}")))?;

        let retention = Arc::new(Mutex::new(RetentionRing::new(
            retain_bytes.unwrap_or(self.default_retain_bytes),
        )));
        let observed_terminal = Arc::new(Mutex::new(ObservedTerminal {
            emulator: TerminalEmulator::new(size),
            revision: 0,
            size,
        }));
        let initial_pump_state = PumpState {
            revision: 0,
            last_output_at: Instant::now(),
            output_closed: false,
        };
        let (pump_state_tx, pump_state) = watch::channel(initial_pump_state);
        let pump_handle = tokio::spawn(run_output_pump(
            read_rx,
            retention.clone(),
            observed_terminal.clone(),
            pump_state_tx,
        ));

        let id = SessionId::new();
        let session = Arc::new(Session {
            id: id.clone(),
            name,
            command,
            cwd,
            created_at: Utc::now(),
            pty,
            retention,
            observed_terminal,
            pump_state,
            pump_task: Mutex::new(PumpTask::new(pump_handle)),
            process_exit: std::sync::Mutex::new(None),
        });

        let mut sessions = self.sessions.write().await;
        if sessions.len() >= MAX_SESSIONS {
            drop(sessions);
            let _output_complete = session.shutdown().await;
            return Err(ApiError::session_limit_reached(MAX_SESSIONS));
        }
        if let Some(ref n) = session.name {
            if sessions.values().any(|s| s.name.as_deref() == Some(n)) {
                drop(sessions);
                let _output_complete = session.shutdown().await;
                return Err(ApiError::duplicate_session_name(n));
            }
        }
        sessions.insert(id.clone(), session);

        Ok(id)
    }

    /// Get a reference to a session by ID.
    ///
    /// Returns an error if the session doesn't exist.
    #[cfg(test)]
    async fn get_session<F, R>(&self, id: &SessionId, f: F) -> Result<R, ApiError>
    where
        F: FnOnce(&Session) -> R,
    {
        let sessions = self.sessions.read().await;
        match sessions.get(id) {
            Some(session) => Ok(f(session)),
            None => Err(ApiError::session_not_found(&id.0)),
        }
    }

    /// Kill a session by ID.
    ///
    /// Returns an error if the session doesn't exist.
    pub async fn kill_session(&self, id: &SessionId) -> Result<(), ApiError> {
        let session = self
            .sessions
            .write()
            .await
            .remove(id)
            .ok_or_else(|| ApiError::session_not_found(&id.0))?;
        let output_complete = session.shutdown().await;
        let tombstone = session
            .final_tombstone(self.next_snapshot_id(), output_complete, true)
            .await;
        self.tombstones
            .lock()
            .await
            .insert(tombstone, Instant::now());
        Ok(())
    }

    /// List all active sessions.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.values().map(|s| s.info()).collect()
    }

    /// Get the number of active sessions.
    #[cfg(test)]
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Check if there are no active sessions.
    pub async fn is_empty(&self) -> bool {
        self.sessions.read().await.is_empty()
    }

    /// Get a session ID by name.
    ///
    /// Returns None if no session with that name exists.
    pub async fn find_by_name(&self, name: &str) -> Option<SessionId> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .find(|s| s.name.as_deref() == Some(name))
            .map(|s| s.id.clone())
    }

    /// Resolve a session identifier to a SessionId.
    ///
    /// The identifier can be:
    /// - None: returns the default session (name: "default")
    /// - A session ID (UUID format)
    /// - A session name
    ///
    /// Returns an error if no matching session is found.
    pub async fn resolve_session(&self, identifier: Option<&str>) -> Result<SessionId, ApiError> {
        let unresolved = match identifier {
            None => {
                // Return default session
                if let Some(id) = self.find_by_name("default").await {
                    return Ok(id);
                }
                "default"
            }
            Some(id_or_name) => {
                let sessions = self.sessions.read().await;
                // First try as session ID
                let id = SessionId::from(id_or_name);
                if sessions.contains_key(&id) {
                    return Ok(id);
                }

                // Then try as session name
                if let Some(id) = sessions
                    .values()
                    .find(|s| s.name.as_deref() == Some(id_or_name))
                    .map(|s| s.id.clone())
                {
                    return Ok(id);
                }
                id_or_name
            }
        };

        match self.resolve_tombstone(unresolved).await {
            Some(tombstone) => Err(ApiError::session_exited(
                unresolved,
                &tombstone.exit.description(),
            )),
            None => Err(ApiError::session_not_found(unresolved)),
        }
    }

    pub(crate) async fn resolve_evidence(
        &self,
        identifier: Option<&str>,
    ) -> Result<SessionEvidence, ApiError> {
        match self.resolve_session(identifier).await {
            Ok(id) => Ok(SessionEvidence::Live(id)),
            Err(error) if error.code == pilotty_core::error::ErrorCode::SessionExited => {
                let identifier = identifier.unwrap_or("default");
                self.resolve_tombstone(identifier)
                    .await
                    .map(Box::new)
                    .map(SessionEvidence::Exited)
                    .ok_or_else(|| ApiError::session_not_found(identifier))
            }
            Err(error) => Err(error),
        }
    }

    async fn resolve_tombstone(&self, identifier: &str) -> Option<Tombstone> {
        let mut tombstones = self.tombstones.lock().await;
        let now = Instant::now();
        tombstones
            .get(&SessionId::from(identifier), now)
            .or_else(|| tombstones.newest_by_name(identifier, now))
    }

    /// Write bytes to a session's PTY.
    pub async fn write_to_session(&self, id: &SessionId, data: &[u8]) -> Result<(), ApiError> {
        let session = self.session(id).await?;

        session
            .write(data)
            .await
            .map_err(|e| ApiError::write_failed(&e.to_string()))
    }

    /// Capture the retained raw output and its exact accounting.
    pub(crate) async fn session_logs(&self, id: &SessionId) -> Result<RetentionSnapshot, ApiError> {
        let session = self.session(id).await?;
        let snapshot = session.retention.lock().await.snapshot();
        Ok(snapshot)
    }

    /// Resize a session's terminal.
    ///
    /// Updates both the PTY size (sends SIGWINCH to child) and the terminal emulator.
    pub async fn resize_session(
        &self,
        id: &SessionId,
        cols: u16,
        rows: u16,
    ) -> Result<(), ApiError> {
        let session = self.session(id).await?;

        let new_size = TermSize { cols, rows };

        // Resize the PTY (sends SIGWINCH to child process)
        session
            .pty
            .resize(new_size)
            .map_err(|e| ApiError::command_failed(format!("Failed to resize PTY: {}", e)))?;

        let mut terminal = session.observed_terminal.lock().await;
        terminal.size = new_size;
        terminal.emulator.resize(new_size);

        Ok(())
    }

    /// Get terminal size for a session.
    pub async fn get_terminal_size(&self, id: &SessionId) -> Result<TermSize, ApiError> {
        let session = self.session(id).await?;
        let size = session.observed_terminal.lock().await.size;
        Ok(size)
    }

    /// Get the application cursor mode for a session.
    ///
    /// The output pump keeps this mode current.
    /// When true, arrow keys should send SS3 sequences instead of CSI.
    pub async fn get_application_cursor_mode(&self, id: &SessionId) -> Result<bool, ApiError> {
        let session = self.session(id).await?;
        Ok(session.application_cursor().await)
    }

    async fn session(&self, id: &SessionId) -> Result<Arc<Session>, ApiError> {
        self.sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ApiError::session_not_found(&id.0))
    }

    /// Subscribe to screen observations for a live session.
    pub(crate) async fn observe_session(
        &self,
        id: &SessionId,
    ) -> Result<SessionObserver, ApiError> {
        let session = self.session(id).await?;
        Ok(SessionObserver {
            pump_state: session.pump_state.clone(),
            session,
        })
    }

    /// Spawn a background task that cleans up dead sessions.
    ///
    /// Periodically checks if child processes have exited and removes their sessions.
    /// This ensures `list-sessions` only shows live sessions and enables idle shutdown
    /// to work correctly (daemon won't stay alive due to dead sessions).
    ///
    /// The task runs until the SessionManager is dropped (via the Arc weak reference).
    pub fn spawn_cleaner(self: &Arc<Self>) {
        let weak_self = Arc::downgrade(self);

        tokio::spawn(async move {
            const CLEANUP_INTERVAL: Duration = Duration::from_millis(500);

            loop {
                tokio::time::sleep(CLEANUP_INTERVAL).await;

                // Try to upgrade weak reference; if it fails, SessionManager was dropped
                let Some(manager) = weak_self.upgrade() else {
                    debug!("SessionManager dropped, cleaner exiting");
                    break;
                };

                // Collect sessions ready to finalize. A direct-process exit starts a
                // bounded drain so a descendant cannot keep the session live forever.
                let finalizing_sessions: Vec<(SessionId, Arc<Session>, bool)> = {
                    let sessions = manager.sessions.read().await;
                    sessions
                        .iter()
                        .filter_map(|(id, session)| {
                            let process_exit = match session.observe_process_exit() {
                                Ok(exit) => exit,
                                Err(error) => {
                                    debug!("Failed to inspect session {}: {}", id, error);
                                    return None;
                                }
                            };
                            let exit = process_exit?;
                            let pump_state = session.pump_state();
                            let output_closed = pump_state.output_closed;
                            let drain_expired = exit.observed_at.elapsed() >= EXIT_DRAIN_TIMEOUT;
                            debug!(
                                "Exit observed for session {} at revision {}, last output {:?} ago",
                                id,
                                pump_state.revision,
                                pump_state.last_output_at.elapsed()
                            );
                            (output_closed || drain_expired)
                                .then(|| (id.clone(), session.clone(), output_closed))
                        })
                        .collect()
                };

                for (id, session, output_complete) in finalizing_sessions {
                    session.finish_pump(output_complete).await;
                    let tombstone = session
                        .final_tombstone(manager.next_snapshot_id(), output_complete, false)
                        .await;
                    let mut tombstones = manager.tombstones.lock().await;
                    let mut sessions = manager.sessions.write().await;
                    let is_same_session = sessions
                        .get(&id)
                        .is_some_and(|current| Arc::ptr_eq(current, &session));
                    if is_same_session {
                        tombstones.insert(tombstone, Instant::now());
                        sessions.remove(&id);
                        info!(
                            "Finalized session {} ({:?}), status: {}, output complete: {}",
                            id,
                            session.name.as_deref().unwrap_or("unnamed"),
                            exit_status_description(&session),
                            output_complete
                        );
                    }
                }

                manager
                    .tombstones
                    .lock()
                    .await
                    .purge_expired(Instant::now());
            }
        });
    }
}

fn exit_status_description(session: &Session) -> String {
    match session.process_exit.lock() {
        Ok(exit) => exit
            .as_ref()
            .map(|exit| exit.status.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        Err(_) => "unavailable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_get_session() {
        let manager = SessionManager::new();

        // Create a session running echo
        let id = manager
            .create_session(
                vec!["echo".to_string(), "hello".to_string()],
                Some("test-session".to_string()),
                None,
                None,
            )
            .await
            .expect("Failed to create session");

        // Verify we can get it
        let info = manager
            .get_session(&id, |s| s.info())
            .await
            .expect("Failed to get session");

        assert_eq!(info.id, id.0);
        assert_eq!(info.name, Some("test-session".to_string()));
        assert_eq!(info.command, vec!["echo", "hello"]);
    }

    #[tokio::test]
    async fn test_kill_session() {
        let manager = SessionManager::new();

        // Create a session
        let id = manager
            .create_session(vec!["cat".to_string()], None, None, None)
            .await
            .expect("Failed to create session");

        assert_eq!(manager.session_count().await, 1);

        // Kill it
        manager
            .kill_session(&id)
            .await
            .expect("Failed to kill session");

        assert_eq!(manager.session_count().await, 0);

        // Getting it should fail
        let result = manager.get_session(&id, |s| s.info()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let manager = SessionManager::new();

        // Create a few sessions
        let _id1 = manager
            .create_session(vec!["echo".to_string(), "1".to_string()], None, None, None)
            .await
            .expect("Failed to create session 1");

        let _id2 = manager
            .create_session(
                vec!["echo".to_string(), "2".to_string()],
                Some("named".to_string()),
                None,
                None,
            )
            .await
            .expect("Failed to create session 2");

        let sessions = manager.list_sessions().await;
        assert_eq!(sessions.len(), 2);

        // Both sessions should have names (default is assigned when omitted)
        let named_count = sessions.iter().filter(|s| s.name.is_some()).count();
        assert_eq!(named_count, 2);
    }

    #[tokio::test]
    async fn test_find_by_name() {
        let manager = SessionManager::new();

        let id = manager
            .create_session(
                vec!["echo".to_string()],
                Some("findme".to_string()),
                None,
                None,
            )
            .await
            .expect("Failed to create session");

        // Find by name should work
        let found = manager.find_by_name("findme").await;
        assert_eq!(found, Some(id));

        // Non-existent name returns None
        let not_found = manager.find_by_name("nope").await;
        assert_eq!(not_found, None);
    }

    #[tokio::test]
    async fn test_session_not_found_error() {
        use pilotty_core::error::ErrorCode;

        let manager = SessionManager::new();

        let fake_id = SessionId::from("nonexistent");
        let result = manager.get_session(&fake_id, |s| s.info()).await;

        match result {
            Err(err) => {
                assert!(matches!(err.code, ErrorCode::SessionNotFound));
                assert!(err.suggestion.is_some());
            }
            Ok(_) => panic!("Expected error for nonexistent session"),
        }
    }

    #[tokio::test]
    async fn test_resolve_session_by_id() {
        let manager = SessionManager::new();

        let id = manager
            .create_session(vec!["echo".to_string()], None, None, None)
            .await
            .expect("Failed to create session");

        // Resolve by ID
        let resolved = manager.resolve_session(Some(&id.0)).await;
        assert_eq!(resolved.unwrap(), id);
    }

    #[tokio::test]
    async fn test_resolve_session_by_name() {
        let manager = SessionManager::new();

        let id = manager
            .create_session(
                vec!["echo".to_string()],
                Some("my-session".to_string()),
                None,
                None,
            )
            .await
            .expect("Failed to create session");

        // Resolve by name
        let resolved = manager.resolve_session(Some("my-session")).await;
        assert_eq!(resolved.unwrap(), id);
    }

    #[tokio::test]
    async fn test_resolve_session_default() {
        let manager = SessionManager::new();

        let id = manager
            .create_session(vec!["echo".to_string(), "1".to_string()], None, None, None)
            .await
            .expect("Failed to create default session");

        // Resolve with None should return default session
        let resolved = manager.resolve_session(None).await;
        assert_eq!(resolved.unwrap(), id);
    }

    #[tokio::test]
    async fn test_resolve_session_no_sessions_error() {
        use pilotty_core::error::ErrorCode;

        let manager = SessionManager::new();

        let result = manager.resolve_session(None).await;
        match result {
            Err(err) => {
                assert!(matches!(err.code, ErrorCode::SessionNotFound));
                assert!(err.suggestion.is_some());
            }
            Ok(_) => panic!("Expected error when no sessions exist"),
        }
    }

    #[tokio::test]
    async fn test_default_session_name_auto_assigned() {
        let manager = SessionManager::new();

        let id = manager
            .create_session(
                vec!["echo".to_string(), "default".to_string()],
                None,
                None,
                None,
            )
            .await
            .expect("Failed to create default session");

        let info = manager
            .get_session(&id, |s| s.info())
            .await
            .expect("Failed to get session info");

        assert_eq!(info.name, Some("default".to_string()));
    }

    #[tokio::test]
    async fn test_duplicate_session_name_error() {
        let manager = SessionManager::new();

        // Create first session with name
        let _id1 = manager
            .create_session(
                vec!["echo".to_string()],
                Some("unique-name".to_string()),
                None,
                None,
            )
            .await
            .expect("Failed to create first session");

        // Try to create another with the same name
        let result = manager
            .create_session(
                vec!["echo".to_string()],
                Some("unique-name".to_string()),
                None,
                None,
            )
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("already exists"),
            "Expected 'already exists' in error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_default_session_duplicate_error() {
        let manager = SessionManager::new();

        let _id1 = manager
            .create_session(vec!["echo".to_string(), "1".to_string()], None, None, None)
            .await
            .expect("Failed to create default session");

        let result = manager
            .create_session(vec!["echo".to_string(), "2".to_string()], None, None, None)
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("already exists"),
            "Expected 'already exists' in error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_cleaner_removes_dead_sessions() {
        let manager = Arc::new(SessionManager::new());

        // Spawn a short-lived process (echo exits immediately)
        let id = manager
            .create_session(
                vec!["echo".to_string(), "goodbye".to_string()],
                None,
                None,
                None,
            )
            .await
            .expect("Failed to create session");

        // Session should exist
        assert_eq!(manager.session_count().await, 1);

        // Start the cleaner
        manager.spawn_cleaner();

        // Give the process time to exit and cleaner to run
        // Echo exits immediately, cleaner runs every 500ms
        tokio::time::sleep(Duration::from_millis(800)).await;

        // Session should be cleaned up
        assert_eq!(
            manager.session_count().await,
            0,
            "Dead session should have been cleaned up"
        );

        // Trying to resolve it should fail
        let result = manager.resolve_session(Some(&id.0)).await;
        assert!(result.is_err(), "Session should no longer exist");
    }

    #[tokio::test]
    async fn test_cleaner_keeps_live_sessions() {
        let manager = Arc::new(SessionManager::new());

        // Spawn a long-lived process (cat waits for input)
        let _id = manager
            .create_session(vec!["cat".to_string()], None, None, None)
            .await
            .expect("Failed to create session");

        // Session should exist
        assert_eq!(manager.session_count().await, 1);

        // Start the cleaner
        manager.spawn_cleaner();

        // Wait for cleaner to run a few times
        tokio::time::sleep(Duration::from_millis(800)).await;

        // Session should still exist (cat is still running)
        assert_eq!(
            manager.session_count().await,
            1,
            "Live session should NOT be cleaned up"
        );
    }

    #[tokio::test]
    async fn verbose_session_exits_without_snapshot_reads() {
        let manager = Arc::new(SessionManager::new());

        manager
            .create_session(
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "yes output | head -c 1048576".to_string(),
                ],
                Some("verbose-session".to_string()),
                None,
                None,
            )
            .await
            .expect("create verbose session");
        manager.spawn_cleaner();

        tokio::time::timeout(Duration::from_secs(3), async {
            while manager.session_count().await != 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("verbose session should exit without requiring snapshot reads");
    }

    #[tokio::test]
    async fn observer_wakes_when_screen_output_arrives() {
        let manager = SessionManager::new();
        let id = manager
            .create_session(
                vec!["cat".to_string()],
                Some("observed-session".to_string()),
                None,
                None,
            )
            .await
            .expect("create observed session");
        let mut observer = manager
            .observe_session(&id)
            .await
            .expect("subscribe to session observations");
        let baseline = observer.current(false).await;

        manager
            .write_to_session(&id, b"observer-marker\n")
            .await
            .expect("write marker");

        assert_eq!(
            observer.wait_for_update(Duration::from_secs(1)).await,
            ObservationEvent::Updated
        );
        let changed = observer.current(false).await;
        assert_ne!(changed.content_hash, baseline.content_hash);
        assert!(changed.revision > baseline.revision);
        assert!(changed.text.contains("observer-marker"));
    }

    #[tokio::test]
    async fn invisible_output_advances_revision_without_changing_screen_hash() {
        let manager = SessionManager::new();
        let id = manager
            .create_session(
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    r"while :; do printf '\033]0;title\007'; sleep 0.02; done".to_string(),
                ],
                Some("invisible-output".to_string()),
                None,
                None,
            )
            .await
            .expect("create invisible-output session");
        let mut observer = manager
            .observe_session(&id)
            .await
            .expect("subscribe to session observations");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let baseline = observer.current(false).await;

        assert_eq!(
            observer.wait_for_update(Duration::from_secs(1)).await,
            ObservationEvent::Updated
        );
        let changed = observer.current(false).await;
        assert!(changed.revision > baseline.revision);
        assert_eq!(changed.content_hash, baseline.content_hash);

        manager.kill_session(&id).await.expect("kill session");
    }

    #[tokio::test]
    async fn output_close_preserves_fast_process_final_screen() {
        let manager = SessionManager::new();
        let id = manager
            .create_session(
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "printf final-screen-sentinel; exit 3".to_string(),
                ],
                Some("final-screen".to_string()),
                None,
                None,
            )
            .await
            .expect("create fast-exit session");
        let mut observer = manager
            .observe_session(&id)
            .await
            .expect("subscribe to session observations");

        loop {
            match observer.wait_for_update(Duration::from_secs(1)).await {
                ObservationEvent::Updated => {}
                ObservationEvent::OutputClosed => break,
                other => panic!("expected output close, got {other:?}"),
            }
        }

        let final_screen = observer.current(false).await;
        assert!(final_screen.text.contains("final-screen-sentinel"));
        manager.kill_session(&id).await.expect("remove session");
    }

    #[tokio::test]
    async fn kill_is_not_blocked_by_saturated_input_writers() {
        let manager = Arc::new(SessionManager::new());
        let id = manager
            .create_session(
                vec!["sleep".to_string(), "10".to_string()],
                Some("blocked-writers".to_string()),
                None,
                None,
            )
            .await
            .expect("create non-reading session");

        let mut writers = Vec::new();
        for _ in 0..96 {
            let manager = manager.clone();
            let id = id.clone();
            writers.push(tokio::spawn(async move {
                let data = b"input-line\n".repeat(8 * 1024);
                manager.write_to_session(&id, &data).await
            }));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;

        tokio::time::timeout(Duration::from_secs(2), manager.kill_session(&id))
            .await
            .expect("kill must not wait for blocked input writers")
            .expect("kill session");

        for writer in writers {
            let _write_result = tokio::time::timeout(Duration::from_secs(1), writer)
                .await
                .expect("input writer should stop after kill")
                .expect("input writer task should not panic");
        }
    }

    #[tokio::test]
    async fn exited_session_finalizes_when_descendant_keeps_pty_open() {
        let manager = Arc::new(SessionManager::new());

        manager
            .create_session(
                vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    "import os,signal,time; signal.signal(signal.SIGHUP, signal.SIG_IGN); pid=os.fork(); os._exit(7) if pid else (os.setsid(), os.write(1, b'descendant-open'), time.sleep(3))".to_string(),
                ],
                Some("inherited-pty".to_string()),
                None,
                None,
            )
            .await
            .expect("create session with descendant");
        manager.spawn_cleaner();

        tokio::time::timeout(Duration::from_millis(2500), async {
            while manager.session_count().await != 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("an exited session must not stay live while a descendant holds the PTY open");

        match manager
            .resolve_evidence(Some("inherited-pty"))
            .await
            .expect("resolve finalized evidence")
        {
            SessionEvidence::Exited(tombstone) => {
                if cfg!(target_os = "linux") {
                    assert!(!tombstone.output_complete);
                }
            }
            SessionEvidence::Live(_) => panic!("session should have finalized"),
        }
    }

    #[tokio::test]
    async fn natural_exit_preserves_final_screen_output_and_status() {
        let manager = Arc::new(SessionManager::new());
        manager
            .create_session(
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "printf final-evidence; exit 7".to_string(),
                ],
                Some("natural-exit".to_string()),
                None,
                Some("/tmp".to_string()),
            )
            .await
            .expect("create exiting session");
        manager.spawn_cleaner();

        let tombstone = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(SessionEvidence::Exited(tombstone)) =
                    manager.resolve_evidence(Some("natural-exit")).await
                {
                    break tombstone;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("session finalization");

        assert_eq!(tombstone.exit.code, Some(7));
        assert!(!tombstone.exit.success);
        assert!(!tombstone.exit.killed_by_client);
        assert!(tombstone.output_complete);
        assert_eq!(tombstone.cwd.as_deref(), Some("/tmp"));
        assert!(tombstone
            .final_screen
            .text
            .as_deref()
            .is_some_and(|text| text.contains("final-evidence")));
        assert!(tombstone.output.bytes.ends_with(b"final-evidence"));
        assert!(manager.is_empty().await);
    }

    #[tokio::test]
    async fn tombstones_do_not_count_as_live_sessions_or_prevent_name_reuse() {
        let manager = SessionManager::new();
        let old_id = manager
            .create_session(
                vec!["cat".to_string()],
                Some("reusable".to_string()),
                None,
                None,
            )
            .await
            .expect("create session");
        manager.kill_session(&old_id).await.expect("kill session");

        assert!(manager.is_empty().await);
        assert!(manager.list_sessions().await.is_empty());

        let new_id = manager
            .create_session(
                vec!["cat".to_string()],
                Some("reusable".to_string()),
                None,
                None,
            )
            .await
            .expect("reuse tombstoned name");

        assert!(matches!(
            manager
                .resolve_evidence(Some("reusable"))
                .await
                .expect("live name wins"),
            SessionEvidence::Live(id) if id == new_id
        ));
        match manager
            .resolve_evidence(Some(&old_id.0))
            .await
            .expect("resolve killed session")
        {
            SessionEvidence::Exited(tombstone) => {
                assert!(tombstone.exit.killed_by_client);
                assert!(tombstone.exit.code.is_some() || tombstone.exit.signal.is_some());
                assert_eq!(tombstone.id, old_id);
            }
            SessionEvidence::Live(_) => panic!("old session should be exited"),
        }
        manager.kill_session(&new_id).await.expect("remove session");
    }

    #[tokio::test]
    async fn configured_default_is_injected_into_new_sessions() {
        let manager = SessionManager::with_default_retain_bytes(4);
        let id = manager
            .create_session_with_retention(
                vec!["printf".to_string(), "abcdef".to_string()],
                Some("default-retention".to_string()),
                None,
                None,
                None,
            )
            .await
            .expect("create session");

        let logs = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let logs = manager.session_logs(&id).await.expect("read logs");
                if logs.total_bytes == 6 {
                    break logs;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("session output");

        assert_eq!(logs.bytes, b"cdef");
        assert_eq!(logs.retained_bytes, 4);
        assert_eq!(logs.dropped_bytes, 2);
        assert!(logs.truncated);
        manager.kill_session(&id).await.expect("remove session");
    }

    #[tokio::test]
    async fn session_override_retains_ordered_raw_output() {
        let manager = SessionManager::with_default_retain_bytes(2);
        let expected = b"\x1b[31mred\x1b[0m";
        let id = manager
            .create_session_with_retention(
                vec![
                    "printf".to_string(),
                    String::from_utf8(expected.to_vec()).expect("valid test bytes"),
                ],
                Some("retention-override".to_string()),
                None,
                None,
                Some(expected.len()),
            )
            .await
            .expect("create session");

        let logs = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let logs = manager.session_logs(&id).await.expect("read logs");
                if logs.total_bytes == expected.len() as u64 {
                    break logs;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("session output");

        assert_eq!(logs.bytes, expected);
        assert_eq!(logs.dropped_bytes, 0);
        assert!(!logs.truncated);
        manager.kill_session(&id).await.expect("remove session");
    }

    #[tokio::test]
    async fn test_is_empty() {
        let manager = SessionManager::new();

        // Initially empty
        assert!(manager.is_empty().await);

        // Create a session
        let id = manager
            .create_session(vec!["cat".to_string()], None, None, None)
            .await
            .expect("Failed to create session");

        // Not empty
        assert!(!manager.is_empty().await);

        // Kill it
        manager.kill_session(&id).await.expect("Failed to kill");

        // Empty again
        assert!(manager.is_empty().await);
    }
}
