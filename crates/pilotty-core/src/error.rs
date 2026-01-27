//! AI-friendly error types with suggestions.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error codes for protocol responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    SessionNotFound,
    CommandFailed,
    InvalidInput,
    InternalError,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::SessionNotFound => write!(f, "SESSION_NOT_FOUND"),
            ErrorCode::CommandFailed => write!(f, "COMMAND_FAILED"),
            ErrorCode::InvalidInput => write!(f, "INVALID_INPUT"),
            ErrorCode::InternalError => write!(f, "INTERNAL_ERROR"),
        }
    }
}

/// An error response with AI-friendly context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub suggestion: Option<String>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, " (hint: {})", suggestion)?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    pub fn session_not_found(session_id: &str) -> Self {
        Self {
            code: ErrorCode::SessionNotFound,
            message: format!("Session '{}' not found", session_id),
            suggestion: Some("Run 'pilotty list-sessions' to see available sessions".into()),
        }
    }

    pub fn command_failed(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::CommandFailed,
            message: message.into(),
            suggestion: Some("Check that the command exists and is executable".into()),
        }
    }

    /// Create a command failed error with stderr output included.
    pub fn command_failed_with_stderr(message: impl Into<String>, stderr: Option<&str>) -> Self {
        let msg = message.into();
        let full_message = match stderr {
            Some(err) if !err.trim().is_empty() => format!("{}\nstderr: {}", msg, err.trim()),
            _ => msg,
        };
        Self {
            code: ErrorCode::CommandFailed,
            message: full_message,
            suggestion: Some("Check that the command exists and has correct arguments".into()),
        }
    }

    /// Create a command failed error with a custom suggestion.
    pub fn command_failed_with_suggestion(
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code: ErrorCode::CommandFailed,
            message: message.into(),
            suggestion: Some(suggestion.into()),
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidInput,
            message: message.into(),
            suggestion: Some("Check the command syntax and try again".into()),
        }
    }

    /// Create an invalid input error with a custom suggestion.
    pub fn invalid_input_with_suggestion(
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code: ErrorCode::InvalidInput,
            message: message.into(),
            suggestion: Some(suggestion.into()),
        }
    }

    pub fn duplicate_session_name(name: &str) -> Self {
        Self {
            code: ErrorCode::InvalidInput,
            message: format!("Session name '{}' already exists", name),
            suggestion: Some(format!(
                "Choose a different name with --name, or kill the existing '{}' session first",
                name
            )),
        }
    }

    pub fn no_sessions() -> Self {
        Self {
            code: ErrorCode::SessionNotFound,
            message: "No active sessions".to_string(),
            suggestion: Some("Run 'pilotty spawn <command>' to create a session".into()),
        }
    }

    /// Create an error when the session limit is reached.
    pub fn session_limit_reached(max: usize) -> Self {
        Self {
            code: ErrorCode::CommandFailed,
            message: format!("Maximum session limit ({}) reached", max),
            suggestion: Some(
                "Kill an existing session with 'pilotty kill' before creating a new one".into(),
            ),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: message.into(),
            suggestion: Some("This is an internal error. Please report it if it persists.".into()),
        }
    }

    /// Create a spawn failed error with context about what went wrong.
    pub fn spawn_failed(command: &[String], error: &str) -> Self {
        let cmd_str = if command.is_empty() {
            "(empty command)".to_string()
        } else {
            command.join(" ")
        };
        Self {
            code: ErrorCode::CommandFailed,
            message: format!("Failed to spawn '{}': {}", cmd_str, error),
            suggestion: Some(format!(
                "Verify '{}' exists in your PATH and is executable. Try running it directly in your terminal first.",
                command.first().map_or("the command", |s| s.as_str())
            )),
        }
    }

    /// Create an error for PTY write failures.
    pub fn write_failed(error: &str) -> Self {
        Self {
            code: ErrorCode::CommandFailed,
            message: format!("Failed to write to terminal: {}", error),
            suggestion: Some(
                "The terminal session may have exited. Run 'pilotty list-sessions' to check."
                    .into(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All error constructors must provide a suggestion.
    /// This is critical for AI-friendly error messages.
    fn assert_has_suggestion(err: &ApiError, context: &str) {
        assert!(
            err.suggestion.is_some(),
            "{} should have a suggestion, but got None",
            context
        );
    }

    #[test]
    fn test_session_not_found_has_suggestion() {
        let err = ApiError::session_not_found("test-123");
        assert_has_suggestion(&err, "session_not_found");
        assert!(err.suggestion.as_ref().unwrap().contains("list-sessions"));
        assert!(err.message.contains("test-123"));
    }

    #[test]
    fn test_command_failed_has_suggestion() {
        let err = ApiError::command_failed("something broke");
        assert_has_suggestion(&err, "command_failed");
        assert!(err.message.contains("something broke"));
    }

    #[test]
    fn test_command_failed_with_stderr() {
        let err = ApiError::command_failed_with_stderr("spawn failed", Some("permission denied"));
        assert_has_suggestion(&err, "command_failed_with_stderr");
        assert!(err.message.contains("spawn failed"));
        assert!(err.message.contains("permission denied"));
    }

    #[test]
    fn test_command_failed_with_stderr_empty() {
        let err = ApiError::command_failed_with_stderr("spawn failed", Some("  "));
        assert_has_suggestion(&err, "command_failed_with_stderr (empty stderr)");
        // Empty stderr should not add "stderr:" to message
        assert!(!err.message.contains("stderr:"));
    }

    #[test]
    fn test_command_failed_with_stderr_none() {
        let err = ApiError::command_failed_with_stderr("spawn failed", None);
        assert_has_suggestion(&err, "command_failed_with_stderr (None)");
        assert!(!err.message.contains("stderr:"));
    }

    #[test]
    fn test_invalid_input_has_suggestion() {
        let err = ApiError::invalid_input("bad argument");
        assert_has_suggestion(&err, "invalid_input");
    }

    #[test]
    fn test_invalid_input_with_custom_suggestion() {
        let err =
            ApiError::invalid_input_with_suggestion("unknown key", "Try Enter, Tab, or Ctrl+C");
        assert_has_suggestion(&err, "invalid_input_with_suggestion");
        assert!(err.suggestion.as_ref().unwrap().contains("Enter"));
    }

    #[test]
    fn test_duplicate_session_name_has_suggestion() {
        let err = ApiError::duplicate_session_name("my-session");
        assert_has_suggestion(&err, "duplicate_session_name");
        assert!(err.message.contains("my-session"));
    }

    #[test]
    fn test_no_sessions_has_suggestion() {
        let err = ApiError::no_sessions();
        assert_has_suggestion(&err, "no_sessions");
        assert!(err.suggestion.as_ref().unwrap().contains("spawn"));
    }

    #[test]
    fn test_internal_has_suggestion() {
        let err = ApiError::internal("unexpected state");
        assert_has_suggestion(&err, "internal");
    }

    #[test]
    fn test_spawn_failed_has_suggestion() {
        let cmd = vec!["vim".to_string(), "file.txt".to_string()];
        let err = ApiError::spawn_failed(&cmd, "command not found");
        assert_has_suggestion(&err, "spawn_failed");
        assert!(err.message.contains("vim file.txt"));
        assert!(err.message.contains("command not found"));
        assert!(err.suggestion.as_ref().unwrap().contains("vim"));
    }

    #[test]
    fn test_spawn_failed_empty_command() {
        let err = ApiError::spawn_failed(&[], "no command");
        assert_has_suggestion(&err, "spawn_failed (empty)");
        assert!(err.message.contains("(empty command)"));
    }

    #[test]
    fn test_write_failed_has_suggestion() {
        let err = ApiError::write_failed("broken pipe");
        assert_has_suggestion(&err, "write_failed");
        assert!(err.message.contains("broken pipe"));
        assert!(err.suggestion.as_ref().unwrap().contains("list-sessions"));
    }

    #[test]
    fn test_display_format_with_suggestion() {
        let err = ApiError::session_not_found("abc");
        let display = format!("{}", err);
        assert!(display.contains("[SESSION_NOT_FOUND]"));
        assert!(display.contains("abc"));
        assert!(display.contains("(hint:"));
    }

    #[test]
    fn test_json_serialization() {
        let err = ApiError::session_not_found("test-session");
        let json = serde_json::to_string(&err).unwrap();

        // Verify all fields are present
        assert!(json.contains("\"code\""));
        assert!(json.contains("\"message\""));
        assert!(json.contains("\"suggestion\""));
        assert!(json.contains("SESSION_NOT_FOUND"));
    }

    #[test]
    fn test_json_deserialization() {
        let json =
            r#"{"code":"SESSION_NOT_FOUND","message":"Session 'x' not found","suggestion":"hint"}"#;
        let err: ApiError = serde_json::from_str(json).unwrap();
        assert!(matches!(err.code, ErrorCode::SessionNotFound));
        assert_eq!(err.message, "Session 'x' not found");
        assert_eq!(err.suggestion, Some("hint".to_string()));
    }
}
