//! Protocol types for CLI-daemon communication.

use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ErrorCode};
use crate::snapshot::ScreenState;

/// Default timeout for snapshot await_change/settle operations (30 seconds).
fn default_snapshot_timeout() -> u64 {
    30000
}

/// Current daemon protocol version.
///
/// Carried on every `Request` and `Response` so a client and an
/// already-running daemon can detect version skew (e.g. mid-upgrade).
/// Messages from binaries that predate versioning deserialize as version 0
/// via `#[serde(default)]`. Bump when a change is incompatible; additive
/// fields and enum variants do not require a bump on their own, but a client
/// must treat a lower daemon version as "this command may not exist yet".
pub const PROTOCOL_VERSION: u32 = 1;

/// Protocol spoken by binaries that predate explicit versioning.
pub const LEGACY_PROTOCOL_VERSION: u32 = 0;

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
            ErrorCode::SessionExited => PROTOCOL_VERSION,
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
        format: Option<SnapshotFormat>,
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
    /// Get the retained raw output for a session.
    Logs { session: Option<String> },
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
            | Self::Logs { .. } => PROTOCOL_VERSION,
            Self::Spawn {
                retain_bytes: None, ..
            }
            | Self::Kill { .. }
            | Self::Snapshot { .. }
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
    /// Full JSON with all metadata including text and elements.
    #[default]
    Full,
    /// Compact format: omits text and elements, just metadata.
    Compact,
    /// Plain text only (no JSON structure).
    Text,
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
    ScreenState(ScreenState),
    /// Text-format snapshot.
    Snapshot {
        format: SnapshotFormat,
        content: String,
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
    /// Bounded raw output retained for a session.
    Logs {
        bytes: Vec<u8>,
        total_bytes: u64,
        retained_bytes: u64,
        dropped_bytes: u64,
        truncated: bool,
    },
}

impl ResponseData {
    /// Oldest protocol that can decode this response payload.
    ///
    /// The exhaustive match prevents a new payload from silently reaching an
    /// older client.
    pub fn minimum_protocol(&self) -> u32 {
        match self {
            Self::Logs { .. } => PROTOCOL_VERSION,
            Self::ScreenState(_)
            | Self::Snapshot { .. }
            | Self::SessionCreated { .. }
            | Self::Sessions { .. }
            | Self::WaitForResult { .. }
            | Self::Ok { .. } => LEGACY_PROTOCOL_VERSION,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_protocol_version() {
        let request = Request::new("req-1", Command::ListSessions);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"protocol\":1"), "got: {json}");
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
        let json = r#"{"id":"req-1","command":{"action":"list_sessions"},"protocol":1,"future_field":true}"#;
        let request: Request = serde_json::from_str(json).unwrap();
        assert_eq!(request.protocol, 1);
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
        assert!(supports_protocol(1, 0));
        assert!(supports_protocol(1, 1));
        assert!(!supports_protocol(0, 1));
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
    fn retention_commands_require_current_protocol() {
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

        assert_eq!(plain_spawn.minimum_protocol(), LEGACY_PROTOCOL_VERSION);
        assert_eq!(configured_spawn.minimum_protocol(), PROTOCOL_VERSION);
        assert_eq!(
            Command::Logs { session: None }.minimum_protocol(),
            PROTOCOL_VERSION
        );
    }

    #[test]
    fn logs_response_requires_current_protocol_and_preserves_raw_bytes() {
        let response = ResponseData::Logs {
            bytes: vec![0, 27, 255],
            total_bytes: 9,
            retained_bytes: 3,
            dropped_bytes: 6,
            truncated: true,
        };

        assert_eq!(response.minimum_protocol(), PROTOCOL_VERSION);
        let json = serde_json::to_string(&response).expect("serialize logs response");
        let decoded: ResponseData = serde_json::from_str(&json).expect("deserialize logs response");
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
