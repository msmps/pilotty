//! PTY session management using portable-pty.

use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

/// Terminal size in columns and rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl Default for TermSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

impl From<TermSize> for PtySize {
    fn from(size: TermSize) -> Self {
        PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// A PTY session wrapping a master PTY and child process.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    size: TermSize,
}

impl PtySession {
    /// Spawn a command in a new PTY session.
    pub fn spawn(command: &[String], size: TermSize) -> Result<Self> {
        if command.is_empty() {
            anyhow::bail!("Command cannot be empty");
        }

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.into())
            .context("Failed to open PTY")?;

        let mut cmd = CommandBuilder::new(&command[0]);
        if command.len() > 1 {
            cmd.args(&command[1..]);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn command")?;

        Ok(Self {
            master: pair.master,
            child,
            size,
        })
    }

    /// Get a reader for the PTY output.
    pub fn reader(&self) -> Result<Box<dyn Read + Send>> {
        self.master
            .try_clone_reader()
            .context("Failed to clone PTY reader")
    }

    /// Get a writer for the PTY input.
    pub fn writer(&self) -> Result<Box<dyn Write + Send>> {
        self.master
            .take_writer()
            .context("Failed to take PTY writer")
    }
    /// Get the current terminal size.
    pub fn size(&self) -> TermSize {
        self.size
    }

    /// Consume the session and return the master PTY and child process.
    ///
    /// Used by AsyncPtyHandle to keep the master for resize operations
    /// and the child for proper process cleanup on shutdown.
    pub fn into_parts(self) -> (Box<dyn MasterPty + Send>, Box<dyn Child + Send + Sync>) {
        (self.master, self.child)
    }
}

/// Buffer size for reading from PTY.
const READ_BUFFER_SIZE: usize = 4096;

/// Handle for async PTY I/O operations.
///
/// Uses tokio channels for async I/O with background threads for the
/// actual blocking PTY reads/writes.
pub struct AsyncPtyHandle {
    /// Sender for writing to PTY stdin.
    write_tx: mpsc::Sender<Vec<u8>>,
    /// Receiver for reading from PTY stdout.
    /// Wrapped in Mutex for interior mutability so read() can take &self,
    /// avoiding the need for &mut self which would require exclusive session access.
    read_rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    /// Flag to signal shutdown.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    /// Master PTY for resize operations (sends SIGWINCH).
    /// Wrapped in Mutex to make AsyncPtyHandle Sync.
    master: std::sync::Mutex<Box<dyn MasterPty + Send>>,
    /// Child process handle for cleanup on shutdown.
    /// Wrapped in Mutex to allow killing from shutdown().
    child: std::sync::Mutex<Box<dyn Child + Send + Sync>>,
    /// Current terminal size, updated on resize.
    /// Wrapped in Mutex for interior mutability.
    size: std::sync::Mutex<TermSize>,
    /// Handle to the reader thread for cleanup.
    reader_thread: Option<std::thread::JoinHandle<()>>,
    /// Handle to the writer thread for cleanup.
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl AsyncPtyHandle {
    /// Create async I/O channels for a PTY session.
    ///
    /// This spawns background threads for reading and writing to the PTY.
    pub fn new(session: PtySession) -> Result<Self> {
        let reader = session.reader()?;
        let writer = session.writer()?;
        let initial_size = session.size();
        let (master, child) = session.into_parts();

        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Create channels
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(64);
        let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>(64);

        // Spawn reader thread
        let reader_shutdown = shutdown.clone();
        let reader_thread = std::thread::spawn(move || {
            Self::reader_loop(reader, read_tx, reader_shutdown);
        });

        // Spawn writer thread
        let writer_thread = std::thread::spawn(move || {
            Self::writer_loop(writer, write_rx);
        });

        Ok(Self {
            write_tx,
            read_rx: tokio::sync::Mutex::new(read_rx),
            shutdown,
            master: std::sync::Mutex::new(master),
            child: std::sync::Mutex::new(child),
            size: std::sync::Mutex::new(initial_size),
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
        })
    }

    /// Resize the PTY and send SIGWINCH to the child process.
    ///
    /// Also updates the internal size tracking. Use `size()` to query.
    pub fn resize(&self, size: TermSize) -> Result<()> {
        self.master
            .lock()
            .map_err(|_| anyhow::anyhow!("Master PTY mutex poisoned"))?
            .resize(size.into())
            .context("Failed to resize PTY")?;
        // Update tracked size
        *self
            .size
            .lock()
            .map_err(|_| anyhow::anyhow!("Size mutex poisoned"))? = size;
        Ok(())
    }
    /// Send bytes to the PTY stdin.
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        self.write_tx
            .send(data.to_vec())
            .await
            .context("Failed to send to PTY input channel")
    }

    /// Receive bytes from the PTY stdout.
    ///
    /// Returns None if the PTY has closed.
    pub async fn read(&self) -> Option<Vec<u8>> {
        self.read_rx.lock().await.recv().await
    }

    /// Check if the child process has exited without blocking.
    ///
    /// Returns `Some(true)` if the process has exited, `Some(false)` if still running,
    /// or `None` if the mutex is poisoned.
    pub fn has_exited(&self) -> Option<bool> {
        self.child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok())
            .map(|status| status.is_some())
    }

    /// Shutdown the async PTY I/O and terminate the child process.
    ///
    /// This kills the child process to prevent orphaned processes,
    /// then signals the I/O threads to stop.
    pub async fn shutdown(&self) {
        // Kill the child process first to prevent orphaned processes
        if let Ok(mut child) = self.child.lock() {
            if let Err(e) = child.kill() {
                // Log but don't fail - process may have already exited
                debug!(
                    "Failed to kill child process (may have already exited): {}",
                    e
                );
            }
            // Collect exit status to prevent zombie process accumulation.
            // This is non-blocking since we just sent SIGKILL.
            if let Err(e) = child.try_wait() {
                debug!("Failed to collect child exit status: {}", e);
            }
        }

        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Close the channels to unblock threads
        self.read_rx.lock().await.close();
    }

    /// Reader loop running in a background thread.
    fn reader_loop(
        mut reader: Box<dyn Read + Send>,
        read_tx: mpsc::Sender<Vec<u8>>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let mut buf = vec![0u8; READ_BUFFER_SIZE];

        loop {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                debug!("PTY reader shutdown");
                break;
            }

            match reader.read(&mut buf) {
                Ok(0) => {
                    debug!("PTY reader EOF");
                    break;
                }
                Ok(n) => {
                    // Use blocking send since we're in a thread
                    if read_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        debug!("PTY read channel closed");
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    warn!("PTY read error: {}", e);
                    break;
                }
            }
        }
    }

    /// Writer loop running in a background thread.
    fn writer_loop(mut writer: Box<dyn Write + Send>, mut write_rx: mpsc::Receiver<Vec<u8>>) {
        // Use blocking_recv since we're in a thread
        while let Some(data) = write_rx.blocking_recv() {
            if let Err(e) = writer.write_all(&data) {
                error!("PTY write error: {}", e);
                break;
            }
            if let Err(e) = writer.flush() {
                error!("PTY flush error: {}", e);
                break;
            }
        }
        debug!("PTY writer exiting");
    }
}

impl Drop for AsyncPtyHandle {
    fn drop(&mut self) {
        // Kill the child process first to prevent orphaned processes.
        // This mirrors the logic in shutdown() but is synchronous since Drop can't be async.
        if let Ok(mut child) = self.child.lock() {
            if let Err(e) = child.kill() {
                // Process may have already exited, that's fine
                debug!(
                    "Failed to kill child on drop (may have already exited): {}",
                    e
                );
            }
            // Collect exit status to prevent zombie accumulation
            if let Err(e) = child.try_wait() {
                debug!("Failed to collect child exit status on drop: {}", e);
            }
        }

        // Signal threads to shutdown
        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Note: We intentionally don't block on join() here because:
        // 1. The reader thread may be blocked on a synchronous read() call
        //    which cannot be interrupted without closing the PTY fd
        // 2. The threads will terminate on their own when:
        //    - Reader: PTY closes (EOF) or channel is dropped
        //    - Writer: Channel closes when write_tx is dropped
        //
        // The thread handles are stored so they're not detached (which would
        // cause issues with thread-local storage), but we don't wait for them.
        // This is acceptable because the threads don't hold any resources that
        // need explicit cleanup - the channels and PTY handles are cleaned up
        // by their own Drop implementations.

        // Log if threads are still running (helpful for debugging)
        if let Some(ref handle) = self.reader_thread {
            if !handle.is_finished() {
                debug!("PTY reader thread still running on drop, will terminate on PTY close");
            }
        }
        if let Some(ref handle) = self.writer_thread {
            if !handle.is_finished() {
                debug!("PTY writer thread still running on drop, will terminate on channel close");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Duration;

    #[test]
    fn test_spawn_echo_and_read_output() {
        let session = PtySession::spawn(
            &["echo".to_string(), "hello".to_string()],
            TermSize::default(),
        )
        .expect("Failed to spawn echo");

        let mut reader = session.reader().expect("Failed to get reader");

        // Read output with timeout
        let mut output = vec![0u8; 1024];
        let mut total_read = 0;

        // Give the process time to write output
        std::thread::sleep(Duration::from_millis(100));

        // Read available data
        loop {
            match reader.read(&mut output[total_read..]) {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    if total_read >= output.len() {
                        break;
                    }
                    // Check if we got our expected output
                    let s = String::from_utf8_lossy(&output[..total_read]);
                    if s.contains("hello") {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let output_str = String::from_utf8_lossy(&output[..total_read]);
        assert!(
            output_str.contains("hello"),
            "Expected 'hello' in output, got: {:?}",
            output_str
        );
    }

    #[test]
    fn test_spawn_and_write_input() {
        // Spawn cat which echoes input
        let session = PtySession::spawn(&["cat".to_string()], TermSize::default())
            .expect("Failed to spawn cat");

        let mut writer = session.writer().expect("Failed to get writer");
        let mut reader = session.reader().expect("Failed to get reader");

        // Write some input
        writer.write_all(b"test input\n").expect("Failed to write");
        writer.flush().expect("Failed to flush");

        // Give it time to echo back
        std::thread::sleep(Duration::from_millis(100));

        // Read the echoed output
        let mut output = vec![0u8; 256];
        let mut total_read = 0;

        loop {
            match reader.read(&mut output[total_read..]) {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    let s = String::from_utf8_lossy(&output[..total_read]);
                    if s.contains("test input") {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let output_str = String::from_utf8_lossy(&output[..total_read]);
        assert!(
            output_str.contains("test input"),
            "Expected 'test input' in output, got: {:?}",
            output_str
        );
    }

    #[tokio::test]
    async fn test_async_pty_bash_exit() {
        // Spawn bash
        let session = PtySession::spawn(&["bash".to_string()], TermSize::default())
            .expect("Failed to spawn bash");

        let handle = AsyncPtyHandle::new(session).expect("Failed to create async handle");

        // Give bash time to start and print prompt
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Drain any initial output (prompt, etc.)
        while let Ok(Some(_)) =
            tokio::time::timeout(Duration::from_millis(100), handle.read()).await
        {
            // Keep reading until no more output
        }

        // Send exit command
        handle.write(b"exit\n").await.expect("Failed to write exit");

        // Wait a bit for bash to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        // The channel should close when bash exits
        // Try to read, should eventually return None or timeout
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while handle.read().await.is_some() {
                // Keep reading until EOF
            }
        })
        .await;

        // Either channel closed or we timed out - both are acceptable
        // since we sent exit and bash should have terminated

        // Shutdown should complete without hanging
        let shutdown_result = tokio::time::timeout(Duration::from_secs(2), handle.shutdown()).await;
        assert!(
            shutdown_result.is_ok(),
            "Shutdown timed out, tasks may be stuck"
        );
    }

    #[tokio::test]
    async fn test_async_pty_handle_resize() {
        // Spawn a shell
        let session =
            PtySession::spawn(&["sh".to_string()], TermSize { cols: 80, rows: 24 }).expect("spawn");

        let handle = AsyncPtyHandle::new(session).expect("async handle");

        // Resize via AsyncPtyHandle (sends SIGWINCH to child process)
        handle
            .resize(TermSize {
                cols: 120,
                rows: 40,
            })
            .expect("resize via async handle should succeed");

        // Resize to smaller
        handle
            .resize(TermSize { cols: 40, rows: 10 })
            .expect("resize to smaller should succeed");
    }
}
