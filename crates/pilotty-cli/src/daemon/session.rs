//! Session manager for tracking active PTY sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};

use pilotty_core::elements::classify::{detect, ClassifyContext};
use pilotty_core::elements::Element;
use pilotty_core::error::ApiError;
use pilotty_core::protocol::SessionInfo;
use pilotty_core::snapshot::compute_content_hash;

use crate::daemon::pty::{AsyncPtyHandle, PtySession, TermSize};
use crate::daemon::terminal::TerminalEmulator;

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

/// Snapshot data returned from `SessionManager::get_snapshot_data`.
pub struct SnapshotData {
    pub text: String,
    pub cursor_pos: (u16, u16),
    pub cursor_visible: bool,
    pub size: TermSize,
    /// Detected UI elements (computed on demand).
    pub elements: Option<Vec<Element>>,
    /// Hash of screen content for change detection.
    /// Present when `with_elements=true`.
    pub content_hash: Option<u64>,
}

/// An active PTY session.
pub struct Session {
    /// Unique session ID.
    pub id: SessionId,
    /// Optional human-readable name.
    pub name: Option<String>,
    /// Command that was spawned.
    pub command: Vec<String>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// Terminal size.
    pub size: TermSize,
    /// Async handle for PTY I/O.
    pub pty: AsyncPtyHandle,
    /// Terminal emulator tracking screen state.
    /// Wrapped in Mutex for interior mutability (fed by PTY reader task).
    pub terminal: Arc<Mutex<TerminalEmulator>>,
}

impl Session {
    /// Get session info for protocol responses.
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.0.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            created_at: self.created_at.to_rfc3339(),
        }
    }

    /// Check if terminal is in application cursor mode.
    pub async fn application_cursor(&self) -> bool {
        self.terminal.lock().await.application_cursor()
    }

    /// Write bytes to the PTY (send input to the terminal).
    pub async fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        self.pty.write(data).await
    }

    /// Drain pending PTY output and feed to terminal emulator.
    ///
    /// Call this before taking a snapshot to ensure screen state is current.
    ///
    /// To prevent blocking on noisy processes, this has limits:
    /// - Maximum 100 iterations (reads)
    /// - Maximum 1MB total data
    /// - 10ms timeout per read
    ///
    /// These limits ensure the drain completes quickly even with high-output processes.
    pub async fn drain_pty_output(&self) {
        use std::time::Duration;
        use tokio::time::timeout;

        // Limits to prevent blocking on noisy processes
        const MAX_ITERATIONS: usize = 100;
        const MAX_BYTES: usize = 1024 * 1024; // 1 MB

        let mut terminal = self.terminal.lock().await;
        let mut iterations = 0;
        let mut total_bytes = 0;

        // Read available data from PTY (non-blocking via short timeout)
        loop {
            if iterations >= MAX_ITERATIONS {
                debug!(
                    "Drain hit iteration limit ({} iterations, {} bytes)",
                    iterations, total_bytes
                );
                break;
            }

            match timeout(Duration::from_millis(10), self.pty.read()).await {
                Ok(Some(data)) => {
                    let len = data.len();
                    debug!("Fed {} bytes to terminal emulator", len);
                    terminal.feed(&data);

                    iterations += 1;
                    total_bytes += len;

                    if total_bytes >= MAX_BYTES {
                        debug!(
                            "Drain hit byte limit ({} iterations, {} bytes)",
                            iterations, total_bytes
                        );
                        break;
                    }
                }
                Ok(None) => {
                    // PTY closed
                    debug!("PTY channel closed");
                    break;
                }
                Err(_) => {
                    // Timeout - no more data available
                    break;
                }
            }
        }
    }
}

/// Maximum number of concurrent sessions to prevent resource exhaustion.
const MAX_SESSIONS: usize = 100;

/// Manages active PTY sessions.
///
/// Thread-safe via interior mutability with RwLock.
pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Session>>,
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
        Self {
            sessions: RwLock::new(HashMap::new()),
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
    pub async fn create_session(
        &self,
        command: Vec<String>,
        name: Option<String>,
        size: Option<TermSize>,
        cwd: Option<String>,
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
            .map_err(|e| ApiError::spawn_failed(&command, &e.to_string()))?;

        // Wrap in async handle
        let pty = AsyncPtyHandle::new(pty_session)
            .map_err(|e| ApiError::spawn_failed(&command, &e.to_string()))?;

        // Create terminal emulator to track screen state
        let terminal = Arc::new(Mutex::new(TerminalEmulator::new(size)));

        let id = SessionId::new();
        let session = Session {
            id: id.clone(),
            name,
            command,
            created_at: Utc::now(),
            size,
            pty,
            terminal,
        };

        let mut sessions = self.sessions.write().await;
        if sessions.len() >= MAX_SESSIONS {
            drop(sessions);
            session.pty.shutdown().await;
            return Err(ApiError::session_limit_reached(MAX_SESSIONS));
        }
        if let Some(ref n) = session.name {
            if sessions.values().any(|s| s.name.as_deref() == Some(n)) {
                drop(sessions);
                session.pty.shutdown().await;
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
    pub async fn get_session<F, R>(&self, id: &SessionId, f: F) -> Result<R, ApiError>
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
        let mut sessions = self.sessions.write().await;
        match sessions.remove(id) {
            Some(session) => {
                // Shutdown the async PTY handle (drops writer, signals reader to stop)
                session.pty.shutdown().await;
                Ok(())
            }
            None => Err(ApiError::session_not_found(&id.0)),
        }
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
        match identifier {
            None => {
                // Return default session
                self.find_by_name("default")
                    .await
                    .ok_or_else(|| ApiError::session_not_found("default"))
            }
            Some(id_or_name) => {
                let sessions = self.sessions.read().await;
                // First try as session ID
                let id = SessionId::from(id_or_name);
                if sessions.contains_key(&id) {
                    return Ok(id);
                }

                // Then try as session name
                sessions
                    .values()
                    .find(|s| s.name.as_deref() == Some(id_or_name))
                    .map(|s| s.id.clone())
                    .ok_or_else(|| ApiError::session_not_found(id_or_name))
            }
        }
    }

    /// Get snapshot data for a session.
    ///
    /// Drains pending PTY output to terminal emulator before capturing snapshot.
    ///
    /// Uses a read lock on sessions since all operations use interior mutability,
    /// avoiding potential deadlocks from holding a write lock during I/O.
    ///
    /// If `with_elements` is true, element detection runs to identify
    /// UI elements like buttons, checkboxes, and menu items.
    pub async fn get_snapshot_data(
        &self,
        id: &SessionId,
        with_elements: bool,
    ) -> Result<SnapshotData, ApiError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(id)
            .ok_or_else(|| ApiError::session_not_found(&id.0))?;

        // Drain pending PTY output to update terminal state
        session.drain_pty_output().await;

        // Lock terminal once for all reads
        let terminal = session.terminal.lock().await;

        // Get snapshot data
        let text = terminal.get_text();
        let cursor_pos = terminal.cursor_position();
        let cursor_visible = terminal.cursor_visible();
        let size = session.size;

        // Detect UI elements and compute content hash if requested
        let (elements, content_hash) = if with_elements {
            let (cursor_row, cursor_col) = cursor_pos;
            let ctx = ClassifyContext::new().with_cursor(cursor_row, cursor_col);
            let elems = detect(&*terminal, &ctx);
            let hash = compute_content_hash(&text);
            (Some(elems), Some(hash))
        } else {
            (None, None)
        };

        Ok(SnapshotData {
            text,
            cursor_pos,
            cursor_visible,
            size,
            elements,
            content_hash,
        })
    }

    /// Write bytes to a session's PTY.
    pub async fn write_to_session(&self, id: &SessionId, data: &[u8]) -> Result<(), ApiError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(id)
            .ok_or_else(|| ApiError::session_not_found(&id.0))?;

        session
            .write(data)
            .await
            .map_err(|e| ApiError::write_failed(&e.to_string()))
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
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| ApiError::session_not_found(&id.0))?;

        let new_size = TermSize { cols, rows };

        // Resize the PTY (sends SIGWINCH to child process)
        session
            .pty
            .resize(new_size)
            .map_err(|e| ApiError::command_failed(format!("Failed to resize PTY: {}", e)))?;

        // Update session's stored size
        session.size = new_size;

        // Resize the terminal emulator
        session.terminal.lock().await.resize(new_size);

        Ok(())
    }

    /// Get terminal size for a session.
    pub async fn get_terminal_size(&self, id: &SessionId) -> Result<TermSize, ApiError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(id)
            .ok_or_else(|| ApiError::session_not_found(&id.0))?;
        Ok(session.size)
    }

    /// Get the application cursor mode for a session.
    ///
    /// Drains pending PTY output first to ensure mode is current.
    /// When true, arrow keys should send SS3 sequences instead of CSI.
    pub async fn get_application_cursor_mode(&self, id: &SessionId) -> Result<bool, ApiError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(id)
            .ok_or_else(|| ApiError::session_not_found(&id.0))?;

        // Drain to get current terminal state
        session.drain_pty_output().await;
        Ok(session.application_cursor().await)
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

                // Collect IDs of sessions with dead processes
                let dead_sessions: Vec<SessionId> = {
                    let sessions = manager.sessions.read().await;
                    sessions
                        .iter()
                        .filter_map(|(id, session)| {
                            // Check if child process has exited
                            if session.pty.has_exited().unwrap_or(false) {
                                Some(id.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                // Remove dead sessions
                if !dead_sessions.is_empty() {
                    let mut sessions = manager.sessions.write().await;
                    for id in dead_sessions {
                        if let Some(session) = sessions.remove(&id) {
                            info!(
                                "Cleaned up session {} ({:?}) - process exited",
                                id,
                                session.name.as_deref().unwrap_or("unnamed")
                            );
                        }
                    }
                }
            }
        });
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
