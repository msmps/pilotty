//! Cross-platform transport for daemon/client communication.
//!
//! Unix uses Unix domain sockets.
//! Windows uses a loopback TCP listener, with the endpoint marker file storing
//! the actual bound port so we can recover from collisions on the preferred one.

use std::io;
use std::path::Path;

#[cfg(unix)]
pub type Listener = tokio::net::UnixListener;
#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;

#[cfg(windows)]
pub type Listener = tokio::net::TcpListener;
#[cfg(windows)]
pub type Stream = tokio::net::TcpStream;

/// Bind a daemon listener for the given endpoint path.
///
/// On Windows this binds the preferred deterministic port derived from the
/// endpoint path. If that port is occupied, callers may use `bind_fallback()`
/// after deciding it is safe to do so.
pub async fn bind(endpoint: &Path) -> io::Result<Listener> {
    #[cfg(unix)]
    {
        tokio::net::UnixListener::bind(endpoint)
    }

    #[cfg(windows)]
    {
        tokio::net::TcpListener::bind(preferred_socket_addr(endpoint)).await
    }
}

/// Bind a fallback daemon listener if the preferred endpoint is unavailable.
#[cfg(unix)]
pub async fn bind_fallback(endpoint: &Path) -> io::Result<Listener> {
    bind(endpoint).await
}

/// Bind a fallback daemon listener if the preferred port is unavailable.
#[cfg(windows)]
pub async fn bind_fallback(endpoint: &Path) -> io::Result<Listener> {
    let preferred = preferred_socket_addr(endpoint);
    let start_port = preferred.port();

    for offset in 1..=u32::from(PORT_SPAN) {
        let next_port = BASE_PORT + (((start_port - BASE_PORT) as u32 + offset) % u32::from(PORT_SPAN)) as u16;
        let addr = std::net::SocketAddr::new(preferred.ip(), next_port);
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => return Ok(listener),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => continue,
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "No free loopback TCP port available for pilotty daemon",
    ))
}

/// Connect to a daemon endpoint.
pub async fn connect(endpoint: &Path) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(endpoint).await
    }

    #[cfg(windows)]
    {
        if let Some(addr) = read_endpoint_marker(endpoint) {
            if let Ok(stream) = tokio::net::TcpStream::connect(addr).await {
                return Ok(stream);
            }
        }

        tokio::net::TcpStream::connect(preferred_socket_addr(endpoint)).await
    }
}

/// Whether an I/O error indicates the endpoint is already in use.
pub fn is_addr_in_use(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::AddrInUse
}

/// Write a small marker file describing the endpoint.
///
/// On Unix the socket itself is the endpoint, so this is a no-op.
/// On Windows we keep a marker file containing the bound loopback address so
/// clients can reconnect even if the daemon had to fall back from the preferred
/// deterministic port.
pub fn write_endpoint_marker(endpoint: &Path, listener: &Listener) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = endpoint;
        let _ = listener;
        Ok(())
    }

    #[cfg(windows)]
    {
        std::fs::write(endpoint, listener.local_addr()?.to_string())
    }
}

/// Return a human-readable endpoint description.
pub fn describe_endpoint(endpoint: &Path) -> String {
    #[cfg(unix)]
    {
        endpoint.display().to_string()
    }

    #[cfg(windows)]
    {
        preferred_socket_addr(endpoint).to_string()
    }
}

/// Return the actual listener address.
#[cfg(unix)]
pub fn describe_listener(_listener: &Listener) -> String {
    "unix-listener".to_string()
}

/// Return the actual listener address.
#[cfg(windows)]
pub fn describe_listener(listener: &Listener) -> String {
    listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "127.0.0.1:<unknown>".to_string())
}

#[cfg(windows)]
const BASE_PORT: u16 = 43000;
#[cfg(windows)]
const PORT_SPAN: u16 = 20_000;

#[cfg(windows)]
fn preferred_socket_addr(endpoint: &Path) -> std::net::SocketAddr {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    let mut hasher = DefaultHasher::new();
    endpoint.to_string_lossy().hash(&mut hasher);
    let offset = (hasher.finish() % u64::from(PORT_SPAN)) as u16;

    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), BASE_PORT + offset)
}

#[cfg(windows)]
fn read_endpoint_marker(endpoint: &Path) -> Option<std::net::SocketAddr> {
    std::fs::read_to_string(endpoint)
        .ok()
        .and_then(|text| text.trim().parse().ok())
}
