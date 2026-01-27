//! Socket and PID file path resolution.
//!
//! Priority for socket directory:
//! 1. `PILOTTY_SOCKET_DIR` (explicit override)
//! 2. `XDG_RUNTIME_DIR/pilotty` (Linux standard)
//! 3. `~/.pilotty` (home directory fallback)
//! 4. `/tmp/pilotty` (last resort)
//!
//! Session support via `PILOTTY_SESSION` env var (default: "default").
//! Each session gets its own socket file: `{socket_dir}/{session}.sock`

use std::env;
use std::path::PathBuf;

/// Get current session name from env or default.
pub fn get_session() -> String {
    env::var("PILOTTY_SESSION").unwrap_or_else(|_| "default".to_string())
}

/// Get socket directory with priority fallback.
///
/// Priority:
/// 1. `PILOTTY_SOCKET_DIR` (explicit override, ignores empty string)
/// 2. `XDG_RUNTIME_DIR/pilotty` (Linux standard, ignores empty string)
/// 3. `~/.pilotty` (home directory fallback)
/// 4. System temp dir (last resort)
pub fn get_socket_dir() -> PathBuf {
    // 1. Explicit override (ignore empty)
    if let Ok(dir) = env::var("PILOTTY_SOCKET_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }

    // 2. XDG_RUNTIME_DIR (Linux standard, ignore empty)
    if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.is_empty() {
            return PathBuf::from(runtime_dir).join("pilotty");
        }
    }

    // 3. Home directory fallback
    if let Some(home) = dirs::home_dir() {
        return home.join(".pilotty");
    }

    // 4. Last resort: temp dir
    env::temp_dir().join("pilotty")
}

/// Validate a session name to prevent path traversal attacks.
///
/// Session names must:
/// - Be non-empty
/// - Contain only alphanumeric characters, hyphens, and underscores
/// - Not start with a hyphen (could be interpreted as option)
///
/// Returns the sanitized name or a safe default if invalid.
pub(crate) fn sanitize_session_name(name: &str) -> String {
    // Check if valid: non-empty, safe chars, doesn't start with hyphen
    let is_valid = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if is_valid {
        name.to_string()
    } else {
        // Log warning and use safe fallback
        tracing::warn!(
            "Invalid session name '{}', using 'default'. Names must contain only alphanumeric, hyphen, underscore.",
            name
        );
        "default".to_string()
    }
}

/// Get socket path for a session.
///
/// If no session is provided, uses the current session from `get_session()`.
/// Session names are sanitized to prevent path traversal.
pub fn get_socket_path(session: Option<&str>) -> PathBuf {
    let sess = session.map(String::from).unwrap_or_else(get_session);
    let safe_sess = sanitize_session_name(&sess);
    get_socket_dir().join(format!("{}.sock", safe_sess))
}

/// Get PID file path for a session.
///
/// If no session is provided, uses the current session from `get_session()`.
/// Session names are sanitized to prevent path traversal.
pub fn get_pid_path(session: Option<&str>) -> PathBuf {
    let sess = session.map(String::from).unwrap_or_else(get_session);
    let safe_sess = sanitize_session_name(&sess);
    get_socket_dir().join(format!("{}.pid", safe_sess))
}

/// Ensure socket directory exists with secure permissions (0700 on Unix).
pub fn ensure_socket_dir() -> std::io::Result<()> {
    let dir = get_socket_dir();
    std::fs::create_dir_all(&dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::daemon::paths::{
        get_session, get_socket_dir, get_socket_path, sanitize_session_name,
    };

    // Mutex to serialize tests that manipulate environment variables.
    // Env var manipulation is inherently non-thread-safe, so tests must run serially.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    // Helper to save and restore env vars during tests.
    // Also holds the mutex guard to ensure serialized access.
    struct EnvGuard {
        vars: Vec<(String, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn new(var_names: &[&str]) -> Self {
            // Lock first to prevent races
            let lock = ENV_MUTEX.lock().unwrap();
            let vars = var_names
                .iter()
                .map(|name| (name.to_string(), std::env::var(name).ok()))
                .collect();
            Self { vars, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.vars {
                // SAFETY: We hold ENV_MUTEX, so no other test thread is modifying env vars
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(name, v),
                        None => std::env::remove_var(name),
                    }
                }
            }
            // _lock is dropped here, releasing the mutex
        }
    }

    #[test]
    fn test_get_session_default() {
        let _guard = EnvGuard::new(&["PILOTTY_SESSION"]);
        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe { std::env::remove_var("PILOTTY_SESSION") };

        assert_eq!(get_session(), "default");
    }

    #[test]
    fn test_get_session_custom() {
        let _guard = EnvGuard::new(&["PILOTTY_SESSION"]);
        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe { std::env::set_var("PILOTTY_SESSION", "my-session") };

        assert_eq!(get_session(), "my-session");
    }

    #[test]
    fn test_get_socket_dir_explicit_override() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::set_var("PILOTTY_SOCKET_DIR", "/custom/socket/path");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        assert_eq!(
            get_socket_dir(),
            std::path::PathBuf::from("/custom/socket/path")
        );
    }

    #[test]
    fn test_get_socket_dir_ignores_empty() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::set_var("PILOTTY_SOCKET_DIR", "");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        // Should fall through to home dir
        assert!(get_socket_dir().to_string_lossy().ends_with(".pilotty"));
    }

    #[test]
    fn test_get_socket_dir_xdg_runtime() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::remove_var("PILOTTY_SOCKET_DIR");
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        }

        assert_eq!(
            get_socket_dir(),
            std::path::PathBuf::from("/run/user/1000/pilotty")
        );
    }

    #[test]
    fn test_get_socket_dir_home_fallback() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::remove_var("PILOTTY_SOCKET_DIR");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let result = get_socket_dir();
        assert!(result.to_string_lossy().ends_with(".pilotty"));
    }

    #[test]
    fn test_get_socket_path_default_session() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "PILOTTY_SESSION", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::set_var("PILOTTY_SOCKET_DIR", "/tmp/test");
            std::env::remove_var("PILOTTY_SESSION");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        assert_eq!(
            get_socket_path(None),
            std::path::PathBuf::from("/tmp/test/default.sock")
        );
    }

    #[test]
    fn test_get_socket_path_custom_session() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "PILOTTY_SESSION", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::set_var("PILOTTY_SOCKET_DIR", "/tmp/test");
            std::env::remove_var("PILOTTY_SESSION");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        assert_eq!(
            get_socket_path(Some("my-session")),
            std::path::PathBuf::from("/tmp/test/my-session.sock")
        );
    }

    #[test]
    fn test_sanitize_valid_names() {
        // Simple alphanumeric
        assert_eq!(sanitize_session_name("default"), "default");
        assert_eq!(sanitize_session_name("session1"), "session1");
        assert_eq!(sanitize_session_name("MySession"), "MySession");

        // With hyphens and underscores
        assert_eq!(sanitize_session_name("my-session"), "my-session");
        assert_eq!(sanitize_session_name("my_session"), "my_session");
        assert_eq!(sanitize_session_name("my-session_123"), "my-session_123");

        // Underscores at start are fine
        assert_eq!(sanitize_session_name("_private"), "_private");
    }

    #[test]
    fn test_sanitize_path_traversal_attacks() {
        // Classic path traversal
        assert_eq!(sanitize_session_name("../../../etc/passwd"), "default");
        assert_eq!(sanitize_session_name(".."), "default");
        assert_eq!(sanitize_session_name("foo/../bar"), "default");

        // Sneaky variants
        assert_eq!(sanitize_session_name("foo/bar"), "default");
        assert_eq!(sanitize_session_name("/etc/passwd"), "default");
        assert_eq!(sanitize_session_name("..\\..\\windows"), "default");
    }

    #[test]
    fn test_sanitize_empty_and_whitespace() {
        assert_eq!(sanitize_session_name(""), "default");
        assert_eq!(sanitize_session_name(" "), "default");
        assert_eq!(sanitize_session_name("  "), "default");
        assert_eq!(sanitize_session_name("\t"), "default");
        assert_eq!(sanitize_session_name("\n"), "default");
    }

    #[test]
    fn test_sanitize_hyphen_at_start() {
        // Hyphens at start could be interpreted as CLI options
        assert_eq!(sanitize_session_name("-session"), "default");
        assert_eq!(sanitize_session_name("--session"), "default");
        assert_eq!(sanitize_session_name("-"), "default");
    }

    #[test]
    fn test_sanitize_special_characters() {
        assert_eq!(sanitize_session_name("session!"), "default");
        assert_eq!(sanitize_session_name("session@home"), "default");
        assert_eq!(sanitize_session_name("session#1"), "default");
        assert_eq!(sanitize_session_name("session$var"), "default");
        assert_eq!(sanitize_session_name("session%20"), "default");
        assert_eq!(sanitize_session_name("session&more"), "default");
        assert_eq!(sanitize_session_name("session;rm -rf"), "default");
        assert_eq!(sanitize_session_name("session|cat"), "default");
        assert_eq!(sanitize_session_name("session`id`"), "default");
        assert_eq!(sanitize_session_name("$(whoami)"), "default");
    }

    #[test]
    fn test_sanitize_null_bytes() {
        assert_eq!(sanitize_session_name("session\0evil"), "default");
        assert_eq!(sanitize_session_name("\0"), "default");
    }

    #[test]
    fn test_socket_path_sanitizes_session() {
        let _guard = EnvGuard::new(&["PILOTTY_SOCKET_DIR", "PILOTTY_SESSION", "XDG_RUNTIME_DIR"]);

        // SAFETY: We hold ENV_MUTEX via _guard
        unsafe {
            std::env::set_var("PILOTTY_SOCKET_DIR", "/tmp/test");
            std::env::remove_var("PILOTTY_SESSION");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        // Path traversal attempt should be sanitized to "default"
        assert_eq!(
            get_socket_path(Some("../../../etc/passwd")),
            std::path::PathBuf::from("/tmp/test/default.sock")
        );
    }
}
