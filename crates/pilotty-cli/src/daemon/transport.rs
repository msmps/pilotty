//! Cross-platform transport for daemon/client communication.
//!
//! Unix uses Unix domain sockets.
//! Windows uses a loopback TCP listener derived from the socket path.

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
pub async fn bind(endpoint: &Path) -> io::Result<Listener> {
    #[cfg(unix)]
    {
        tokio::net::UnixListener::bind(endpoint)
    }

    #[cfg(windows)]
    {
        tokio::net::TcpListener::bind(socket_addr(endpoint)).await
    }
}

/// Connect to a daemon endpoint.
pub async fn connect(endpoint: &Path) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(endpoint).await
    }

    #[cfg(windows)]
    {
        tokio::net::TcpStream::connect(socket_addr(endpoint)).await
    }
}

/// Whether an I/O error indicates the endpoint is already in use.
pub fn is_addr_in_use(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::AddrInUse
}

/// Write a small marker file describing the endpoint.
///
/// On Unix the socket itself is the endpoint, so this is a no-op.
/// On Windows we keep a marker file so the existing path-based cleanup logic
/// still has something tangible to remove and inspect.
pub fn write_endpoint_marker(endpoint: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = endpoint;
        Ok(())
    }

    #[cfg(windows)]
    {
        std::fs::write(endpoint, socket_addr(endpoint).to_string())
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
        socket_addr(endpoint).to_string()
    }
}

#[cfg(windows)]
fn socket_addr(endpoint: &Path) -> std::net::SocketAddr {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    const BASE_PORT: u16 = 43000;
    const PORT_SPAN: u16 = 20_000;

    let mut hasher = DefaultHasher::new();
    endpoint.to_string_lossy().hash(&mut hasher);
    let offset = (hasher.finish() % u64::from(PORT_SPAN)) as u16;

    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), BASE_PORT + offset)
}
