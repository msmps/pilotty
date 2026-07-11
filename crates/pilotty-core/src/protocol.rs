//! Protocol types for CLI-daemon communication.

use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ErrorCode};
use crate::snapshot::{ScreenState, TerminalSize};

/// Default timeout for snapshot await_change/settle operations (30 seconds).
fn default_snapshot_timeout() -> u64 {
    30000
}

/// Protocol spoken by binaries that predate explicit versioning.
pub const LEGACY_PROTOCOL_VERSION: u32 = 0;

/// Versioned envelope shipped in v0.0.8.
pub const PROTOCOL_V1: u32 = 1;

/// Bounded retention and finalized-session evidence introduced for v0.0.9.
pub const PROTOCOL_V2: u32 = 2;

/// Minimal text-and-cursor screen snapshots.
pub const PROTOCOL_V3: u32 = 3;

/// Current daemon protocol advertised on every request and response.
///
/// Historical minimum-version mappings below must use the stable version
/// constants, not this moving alias.
pub const PROTOCOL_VERSION: u32 = PROTOCOL_V3;

/// Whether an observed peer protocol satisfies a wire variant's requirement.
pub fn supports_protocol(observed: u32, required: u32) -> bool {
    observed >= required
}

impl ApiError {
    /// Oldest protocol that can decode this error without losing meaning.
    ///
    /// Keeping the exhaustive error-code mapping with the other wire policy
    /// makes a new code declare its compatibility before it can compile.
    pub fn minimum_protocol(&self) -> u32 {
        match &self.code {
            ErrorCode::SessionNotFound
            | ErrorCode::CommandFailed
            | ErrorCode::InvalidInput
            | ErrorCode::InternalError => LEGACY_PROTOCOL_VERSION,
            ErrorCode::SessionExited => PROTOCOL_V2,
        }
    }
}

/// A request from CLI to daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub command: Command,
    /// Protocol version spoken by the client. 0 = predates versioning.
    #[serde(default)]
    pub protocol: u32,
}

impl Request {
    pub fn new(id: impl Into<String>, command: Command) -> Self {
        Self {
            id: id.into(),
            command,
            protocol: PROTOCOL_VERSION,
        }
    }
}

/// Commands the daemon can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    /// Spawn a new PTY session.
    Spawn {
        command: Vec<String>,
        session_name: Option<String>,
        /// Working directory for the spawned process.
        ///
        /// The CLI defaults this to the client's current directory. The path
        /// must be an existing directory. If not provided by the client, the
        /// process inherits the daemon's working directory.
        cwd: Option<String>,
        /// Maximum raw output bytes retained for this session.
        /// Uses the daemon default when omitted.
        #[serde(default)]
        retain_bytes: Option<u64>,
    },
    /// Kill a session.
    Kill { session: Option<String> },
    /// Get a snapshot of the terminal screen.
    ///
    /// Optionally block until the screen changes from a baseline hash and/or
    /// stabilizes for a specified duration.
    Snapshot {
        session: Option<String>,
        #[serde(default)]
        format: SnapshotFormat,
        /// If set, block until content_hash differs from this value.
        #[serde(default)]
        await_change: Option<u64>,
        /// Wait for screen to be stable for this many ms before returning.
        #[serde(default)]
        settle_ms: u64,
        /// Timeout in ms for await_change/settle operations.
        #[serde(default = "default_snapshot_timeout")]
        timeout_ms: u64,
    },
    /// Type text at cursor.
    Type {
        text: String,
        session: Option<String>,
    },
    /// Send a key, key combo, or key sequence.
    ///
    /// For sequences (space-separated keys like "Ctrl+X m"), `delay_ms` specifies
    /// the delay between each key. Defaults to 0 (no delay). Maximum is 10000ms.
    Key {
        key: String,
        /// Delay between keys in a sequence (milliseconds). Defaults to 0, max 10000.
        #[serde(default)]
        delay_ms: u32,
        session: Option<String>,
    },
    /// Click at a specific row/column coordinate.
    Click {
        row: u16,
        col: u16,
        session: Option<String>,
    },
    /// Scroll the terminal.
    Scroll {
        direction: ScrollDirection,
        amount: u32,
        session: Option<String>,
    },
    /// List all active sessions.
    ListSessions,
    /// Get readable retained output or exact ANSI/VT bytes for a session.
    Output {
        session: Option<String>,
        #[serde(default)]
        ansi: bool,
    },
    /// Get live or finalized lifecycle status for a session.
    Status { session: Option<String> },
    /// Resize the terminal.
    Resize {
        cols: u16,
        rows: u16,
        session: Option<String>,
    },
    /// Wait for text to appear.
    WaitFor {
        pattern: String,
        timeout_ms: Option<u64>,
        regex: Option<bool>,
        session: Option<String>,
    },
    /// Shutdown the daemon gracefully.
    Shutdown,
}

impl Command {
    /// Oldest protocol that can decode and honor this command.
    ///
    /// Keep this match exhaustive so every new command must declare its wire
    /// compatibility before it can compile.
    pub fn minimum_protocol(&self) -> u32 {
        match self {
            Self::Spawn {
                retain_bytes: Some(_),
                ..
            }
            | Self::Output { .. }
            | Self::Status { .. } => PROTOCOL_V2,
            Self::Snapshot { .. } => PROTOCOL_V3,
            Self::Spawn {
                retain_bytes: None, ..
            }
            | Self::Kill { .. }
            | Self::Type { .. }
            | Self::Key { .. }
            | Self::Click { .. }
            | Self::Scroll { .. }
            | Self::ListSessions
            | Self::Resize { .. }
            | Self::WaitFor { .. }
            | Self::Shutdown => LEGACY_PROTOCOL_VERSION,
        }
    }
}

/// Snapshot output format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormat {
    /// Full JSON with screen text and content hash.
    #[default]
    Full,
    /// Compact format: omits text and content hash, just metadata.
    Compact,
    /// Plain text only (no JSON structure).
    Text,
}

/// Representation returned by the output command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    Ansi,
}

/// How an outcome-aware snapshot completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOutcome {
    Immediate,
    Settled,
    Changed,
    Deadline,
    Exited,
}

/// Process evidence attached when a capture observes an exited session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureExit {
    pub exit_code: Option<u32>,
    pub signal: Option<String>,
    pub success: bool,
    pub killed_by_client: bool,
    pub output_complete: bool,
}

/// A screen capture with optional wait and lifecycle evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenCapture {
    #[serde(flatten)]
    pub screen: ScreenState,
    pub outcome: CaptureOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<CaptureExit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Scroll direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
}

/// A response from daemon to CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    /// Protocol version spoken by the daemon. 0 = predates versioning.
    #[serde(default)]
    pub protocol: u32,
}

impl Response {
    pub fn success(id: impl Into<String>, data: ResponseData) -> Self {
        Self {
            id: id.into(),
            success: true,
            data: Some(data),
            error: None,
            protocol: PROTOCOL_VERSION,
        }
    }

    pub fn error(id: impl Into<String>, error: ApiError) -> Self {
        Self {
            id: id.into(),
            success: false,
            data: None,
            error: Some(error),
            protocol: PROTOCOL_VERSION,
        }
    }

    /// Oldest protocol that can decode this response without losing meaning.
    pub fn minimum_protocol(&self) -> u32 {
        let data_protocol = self
            .data
            .as_ref()
            .map(ResponseData::minimum_protocol)
            .unwrap_or(LEGACY_PROTOCOL_VERSION);
        let error_protocol = self
            .error
            .as_ref()
            .map(ApiError::minimum_protocol)
            .unwrap_or(LEGACY_PROTOCOL_VERSION);
        data_protocol.max(error_protocol)
    }
}

/// Response payload variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseData {
    /// Full screen state snapshot.
    ScreenState(ScreenCapture),
    /// Text-format snapshot.
    Snapshot {
        format: SnapshotFormat,
        content: String,
        outcome: CaptureOutcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit: Option<CaptureExit>,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Session created response.
    SessionCreated { session_id: String, message: String },
    /// List of active sessions.
    Sessions { sessions: Vec<SessionInfo> },
    /// Wait-for result with match info.
    WaitForResult {
        found: bool,
        matched_text: Option<String>,
        elapsed_ms: u64,
    },
    /// Generic success message.
    Ok { message: String },
    /// Readable or exact output derived from a session's bounded retention window.
    Output {
        format: OutputFormat,
        bytes: Vec<u8>,
        total_bytes: u64,
        retained_bytes: u64,
        dropped_bytes: u64,
        truncated: bool,
    },
    /// Live or finalized lifecycle status for a session.
    Status(SessionStatus),
}

impl ResponseData {
    /// Oldest protocol that can decode this response payload.
    ///
    /// The exhaustive match prevents a new payload from silently reaching an
    /// older client.
    pub fn minimum_protocol(&self) -> u32 {
        match self {
            Self::ScreenState(_) | Self::Snapshot { .. } => PROTOCOL_V3,
            Self::Output { .. } | Self::Status(_) => PROTOCOL_V2,
            Self::SessionCreated { .. }
            | Self::Sessions { .. }
            | Self::WaitForResult { .. }
            | Self::Ok { .. } => LEGACY_PROTOCOL_VERSION,
        }
    }

    /// Return the outcome attached to a screen or text capture.
    pub fn capture_outcome(&self) -> Option<CaptureOutcome> {
        match self {
            Self::ScreenState(capture) => Some(capture.outcome),
            Self::Snapshot { outcome, .. } => Some(*outcome),
            _ => None,
        }
    }
}

/// Information about an active session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub command: Vec<String>,
    pub created_at: String,
}

/// Exact accounting for a session's bounded raw-output evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionAccounting {
    pub total_bytes: u64,
    pub retained_bytes: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
}

/// Lifecycle status and available evidence for a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStatus {
    Running {
        id: String,
        name: Option<String>,
        command: Vec<String>,
        cwd: Option<String>,
        created_at: String,
        size: TerminalSize,
        idle_ms: u64,
        retention: RetentionAccounting,
    },
    Exited {
        id: String,
        name: Option<String>,
        command: Vec<String>,
        cwd: Option<String>,
        created_at: String,
        ended_at: String,
        size: TerminalSize,
        exit_code: Option<u32>,
        signal: Option<String>,
        success: bool,
        killed_by_client: bool,
        output_complete: bool,
        retention: RetentionAccounting,
    },
}

#[cfg(test)]
mod tests {
    use crate::protocol::*;

    #[test]
    fn request_serializes_with_protocol_version() {
        let request = Request::new("req-1", Command::ListSessions);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"protocol\":3"), "got: {json}");
    }

    #[test]
    fn legacy_request_without_protocol_deserializes_as_version_zero() {
        // Wire format sent by clients that predate protocol versioning.
        let json = r#"{"id":"req-1","command":{"action":"list_sessions"}}"#;
        let request: Request = serde_json::from_str(json).unwrap();
        assert_eq!(request.protocol, 0);
        assert_eq!(request.id, "req-1");
    }

    #[test]
    fn legacy_daemon_ignores_unknown_protocol_field() {
        // A current client's request must still parse if a field is unknown
        // to the receiver; serde ignores unknown fields by default. Guard
        // against someone adding deny_unknown_fields later.
        let json = r#"{"id":"req-1","command":{"action":"list_sessions"},"protocol":3,"future_field":true}"#;
        let request: Request = serde_json::from_str(json).unwrap();
        assert_eq!(request.protocol, PROTOCOL_V3);
    }

    #[test]
    fn response_constructors_set_protocol_version() {
        let ok = Response::success(
            "req-1",
            ResponseData::Ok {
                message: "done".to_string(),
            },
        );
        assert_eq!(ok.protocol, PROTOCOL_VERSION);

        let err = Response::error("req-2", ApiError::session_not_found("nope"));
        assert_eq!(err.protocol, PROTOCOL_VERSION);
    }

    #[test]
    fn legacy_response_without_protocol_deserializes_as_version_zero() {
        // Wire format sent by daemons that predate protocol versioning:
        // the client uses version 0 to detect a stale running daemon.
        let json = r#"{"id":"req-1","success":true}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        assert_eq!(response.protocol, 0);
    }

    #[test]
    fn legacy_client_ignores_unknown_response_protocol_field() {
        #[derive(Deserialize)]
        struct LegacyResponse {
            id: String,
            success: bool,
        }

        let response = Response::success(
            "req-1",
            ResponseData::Ok {
                message: "done".to_string(),
            },
        );
        let json = serde_json::to_string(&response).expect("serialize current response");
        let legacy: LegacyResponse =
            serde_json::from_str(&json).expect("legacy client should ignore added fields");

        assert_eq!(legacy.id, "req-1");
        assert!(legacy.success);
    }

    #[test]
    fn protocol_support_is_monotonic() {
        assert!(supports_protocol(PROTOCOL_V2, LEGACY_PROTOCOL_VERSION));
        assert!(supports_protocol(PROTOCOL_V2, PROTOCOL_V1));
        assert!(supports_protocol(PROTOCOL_V2, PROTOCOL_V2));
        assert!(!supports_protocol(PROTOCOL_V1, PROTOCOL_V2));
        assert!(supports_protocol(PROTOCOL_V3, PROTOCOL_V2));
        assert!(supports_protocol(PROTOCOL_V3, PROTOCOL_V3));
        assert!(!supports_protocol(PROTOCOL_V2, PROTOCOL_V3));
    }

    #[test]
    fn existing_wire_variants_remain_legacy_compatible() {
        let command = Command::ListSessions;
        let response = Response::success(
            "req-1",
            ResponseData::Ok {
                message: "done".to_string(),
            },
        );

        assert_eq!(command.minimum_protocol(), 0);
        assert_eq!(response.minimum_protocol(), 0);
    }

    #[test]
    fn versioned_wire_variants_declare_minimum_protocol() {
        let plain_spawn = Command::Spawn {
            command: vec!["sh".to_string()],
            session_name: None,
            cwd: None,
            retain_bytes: None,
        };
        let configured_spawn = Command::Spawn {
            command: vec!["sh".to_string()],
            session_name: None,
            cwd: None,
            retain_bytes: Some(1024),
        };
        let status = SessionStatus::Running {
            id: "session-1".to_string(),
            name: Some("editor".to_string()),
            command: vec!["vi".to_string(), "notes.txt".to_string()],
            cwd: Some("/tmp".to_string()),
            created_at: "2026-07-11T12:00:00Z".to_string(),
            size: TerminalSize {
                cols: 120,
                rows: 40,
            },
            idle_ms: 250,
            retention: RetentionAccounting {
                total_bytes: 12,
                retained_bytes: 10,
                dropped_bytes: 2,
                truncated: true,
            },
        };
        let outcome_snapshot = Command::Snapshot {
            session: None,
            format: SnapshotFormat::Full,
            await_change: None,
            settle_ms: 0,
            timeout_ms: 30_000,
        };

        assert_eq!(plain_spawn.minimum_protocol(), LEGACY_PROTOCOL_VERSION);
        assert_eq!(configured_spawn.minimum_protocol(), PROTOCOL_V2);
        assert_eq!(outcome_snapshot.minimum_protocol(), PROTOCOL_V3);
        assert_eq!(
            Command::Output {
                session: None,
                ansi: false,
            }
            .minimum_protocol(),
            PROTOCOL_V2
        );
        assert_eq!(
            Command::Status { session: None }.minimum_protocol(),
            PROTOCOL_V2
        );
        assert_eq!(ResponseData::Status(status).minimum_protocol(), PROTOCOL_V2);
    }

    #[test]
    fn capture_outcome_round_trips_as_flat_snapshot_evidence() {
        let response = ResponseData::ScreenState(ScreenCapture {
            screen: ScreenState::empty(80, 24),
            outcome: CaptureOutcome::Deadline,
            exit: None,
            note: Some("Screen kept changing".to_string()),
        });

        assert_eq!(response.minimum_protocol(), PROTOCOL_V3);
        let json = serde_json::to_string(&response).expect("serialize capture outcome");
        assert!(json.contains("\"type\":\"screen_state\""), "got: {json}");
        assert!(json.contains("\"outcome\":\"deadline\""), "got: {json}");
        assert!(!json.contains("\"screen\""), "got: {json}");

        let decoded: ResponseData =
            serde_json::from_str(&json).expect("deserialize capture outcome");
        assert_eq!(decoded, response);
    }

    #[test]
    fn output_response_requires_protocol_v2_and_preserves_raw_bytes() {
        let response = ResponseData::Output {
            format: OutputFormat::Ansi,
            bytes: vec![0, 27, 255],
            total_bytes: 9,
            retained_bytes: 3,
            dropped_bytes: 6,
            truncated: true,
        };

        assert_eq!(response.minimum_protocol(), PROTOCOL_V2);
        let json = serde_json::to_string(&response).expect("serialize output response");
        let decoded: ResponseData =
            serde_json::from_str(&json).expect("deserialize output response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn exited_status_round_trips_with_stable_state_and_evidence_fields() {
        let response = ResponseData::Status(SessionStatus::Exited {
            id: "session-1".to_string(),
            name: Some("job".to_string()),
            command: vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
            cwd: None,
            created_at: "2026-07-11T12:00:00Z".to_string(),
            ended_at: "2026-07-11T12:00:01Z".to_string(),
            size: TerminalSize { cols: 80, rows: 24 },
            exit_code: Some(7),
            signal: None,
            success: false,
            killed_by_client: false,
            output_complete: true,
            retention: RetentionAccounting {
                total_bytes: 70_000,
                retained_bytes: 65_536,
                dropped_bytes: 4_464,
                truncated: true,
            },
        });

        let json = serde_json::to_string(&response).expect("serialize exited status");
        assert!(json.contains("\"type\":\"status\""), "got: {json}");
        assert!(json.contains("\"state\":\"exited\""), "got: {json}");
        let decoded: ResponseData = serde_json::from_str(&json).expect("deserialize exited status");
        assert_eq!(decoded, response);
    }

    #[test]
    fn legacy_spawn_without_retention_field_uses_daemon_default() {
        let json = r#"{"action":"spawn","command":["sh"],"session_name":null,"cwd":null}"#;
        let command: Command = serde_json::from_str(json).expect("deserialize legacy spawn");

        assert_eq!(
            command,
            Command::Spawn {
                command: vec!["sh".to_string()],
                session_name: None,
                cwd: None,
                retain_bytes: None,
            }
        );
    }
}
