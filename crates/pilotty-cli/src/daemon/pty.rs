//! PTY / process session management.

use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

#[cfg(unix)]
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

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

#[cfg(unix)]
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

/// A spawned terminal/process session.
pub struct PtySession {
    size: TermSize,
    #[cfg(unix)]
    master: Box<dyn MasterPty + Send>,
    #[cfg(unix)]
    child: Box<dyn Child + Send + Sync>,
    #[cfg(windows)]
    reader: Option<Box<dyn Read + Send>>,
    #[cfg(windows)]
    writer: Option<Box<dyn Write + Send>>,
    #[cfg(windows)]
    child: std::process::Child,
}

impl PtySession {
    /// Spawn a command in a new PTY/session.
    pub fn spawn(command: &[String], size: TermSize, cwd: Option<&str>) -> Result<Self> {
        if command.is_empty() {
            anyhow::bail!("Command cannot be empty");
        }

        #[cfg(unix)]
        {
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(size.into())
                .context("Failed to open PTY")?;

            let mut cmd = CommandBuilder::new(&command[0]);
            if command.len() > 1 {
                cmd.args(&command[1..]);
            }
            if let Some(dir) = cwd {
                cmd.cwd(dir);
            }

            let child = pair
                .slave
                .spawn_command(cmd)
                .context("Failed to spawn command")?;

            return Ok(Self {
                size,
                master: pair.master,
                child,
            });
        }

        #[cfg(windows)]
        {
            let mut cmd = std::process::Command::new(&command[0]);
            if command.len() > 1 {
                cmd.args(&command[1..]);
            }
            if let Some(dir) = cwd {
                cmd.current_dir(dir);
            }

            cmd.stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());

            let mut child = cmd.spawn().context("Failed to spawn command")?;
            let reader = child
                .stdout
                .take()
                .context("Failed to capture process stdout")?;
            let writer = child
                .stdin
                .take()
                .context("Failed to capture process stdin")?;

            return Ok(Self {
                size,
                reader: Some(Box::new(reader)),
                writer: Some(Box::new(writer)),
                child,
            });
        }
    }

    #[cfg(unix)]
    pub fn reader(&self) -> Result<Box<dyn Read + Send>> {
        self.master
            .try_clone_reader()
            .context("Failed to clone PTY reader")
    }

    #[cfg(unix)]
    pub fn writer(&self) -> Result<Box<dyn Write + Send>> {
        self.master
            .take_writer()
            .context("Failed to take PTY writer")
    }

    #[cfg(windows)]
    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>> {
        self.reader
            .take()
            .context("Process reader already taken")
    }

    #[cfg(windows)]
    fn take_writer(&mut self) -> Result<Box<dyn Write + Send>> {
        self.writer
            .take()
            .context("Process writer already taken")
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    #[cfg(unix)]
    pub fn into_parts(self) -> (Box<dyn MasterPty + Send>, ManagedChild) {
        (self.master, ManagedChild::Portable(self.child))
    }

    #[cfg(windows)]
    fn into_child(self) -> ManagedChild {
        ManagedChild::Std(self.child)
    }
}

enum ManagedChild {
    #[cfg(unix)]
    Portable(Box<dyn Child + Send + Sync>),
    #[cfg(windows)]
    Std(std::process::Child),
}

impl ManagedChild {
    fn kill(&mut self) -> Result<()> {
        match self {
            #[cfg(unix)]
            Self::Portable(child) => child.kill().context("Failed to kill PTY child"),
            #[cfg(windows)]
            Self::Std(child) => child.kill().context("Failed to kill process child"),
        }
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        match self {
            #[cfg(unix)]
            Self::Portable(child) => child.try_wait().context("Failed to poll PTY child"),
            #[cfg(windows)]
            Self::Std(child) => child.try_wait().context("Failed to poll process child"),
        }
    }
}

/// Buffer size for reading from PTY/process stdout.
const READ_BUFFER_SIZE: usize = 4096;

/// Handle for async terminal/process I/O operations.
pub struct AsyncPtyHandle {
    write_tx: mpsc::Sender<Vec<u8>>,
    read_rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(unix)]
    master: std::sync::Mutex<Box<dyn MasterPty + Send>>,
    child: std::sync::Mutex<ManagedChild>,
    size: std::sync::Mutex<TermSize>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl AsyncPtyHandle {
    /// Create async I/O channels for a PTY/process session.
    pub fn new(session: PtySession) -> Result<Self> {
        #[cfg(unix)]
        {
            let reader = session.reader()?;
            let writer = session.writer()?;
            let initial_size = session.size();
            let (master, child) = session.into_parts();
            return Self::new_inner(reader, writer, initial_size, Some(master), child);
        }

        #[cfg(windows)]
        {
            let mut session = session;
            let reader = session.take_reader()?;
            let writer = session.take_writer()?;
            let initial_size = session.size();
            let child = session.into_child();
            return Self::new_inner(reader, writer, initial_size, child);
        }
    }

    #[cfg(unix)]
    fn new_inner(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        initial_size: TermSize,
        master: Option<Box<dyn MasterPty + Send>>,
        child: ManagedChild,
    ) -> Result<Self> {
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(64);
        let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>(64);

        let reader_shutdown = shutdown.clone();
        let reader_thread = std::thread::spawn(move || {
            Self::reader_loop(reader, read_tx, reader_shutdown);
        });

        let writer_thread = std::thread::spawn(move || {
            Self::writer_loop(writer, write_rx);
        });

        Ok(Self {
            write_tx,
            read_rx: tokio::sync::Mutex::new(read_rx),
            shutdown,
            master: std::sync::Mutex::new(master.context("Missing PTY master")?),
            child: std::sync::Mutex::new(child),
            size: std::sync::Mutex::new(initial_size),
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
        })
    }

    #[cfg(windows)]
    fn new_inner(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        initial_size: TermSize,
        child: ManagedChild,
    ) -> Result<Self> {
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(64);
        let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>(64);

        let reader_shutdown = shutdown.clone();
        let reader_thread = std::thread::spawn(move || {
            Self::reader_loop(reader, read_tx, reader_shutdown);
        });

        let writer_thread = std::thread::spawn(move || {
            Self::writer_loop(writer, write_rx);
        });

        Ok(Self {
            write_tx,
            read_rx: tokio::sync::Mutex::new(read_rx),
            shutdown,
            child: std::sync::Mutex::new(child),
            size: std::sync::Mutex::new(initial_size),
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
        })
    }

    /// Resize the PTY. On Windows pipe-backed sessions this is currently a no-op.
    pub fn resize(&self, size: TermSize) -> Result<()> {
        #[cfg(unix)]
        {
            self.master
                .lock()
                .map_err(|_| anyhow::anyhow!("Master PTY mutex poisoned"))?
                .resize(size.into())
                .context("Failed to resize PTY")?;
        }

        *self
            .size
            .lock()
            .map_err(|_| anyhow::anyhow!("Size mutex poisoned"))? = size;
        Ok(())
    }

    /// Send bytes to stdin.
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        self.write_tx
            .send(data.to_vec())
            .await
            .context("Failed to send to input channel")
    }

    /// Receive bytes from stdout.
    pub async fn read(&self) -> Option<Vec<u8>> {
        self.read_rx.lock().await.recv().await
    }

    /// Check whether the child has exited.
    pub fn has_exited(&self) -> Option<bool> {
        self.child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok())
            .map(|status| status.is_some())
    }

    /// Shutdown async I/O and terminate the child process.
    pub async fn shutdown(&self) {
        if let Ok(mut child) = self.child.lock() {
            if let Err(e) = child.kill() {
                debug!("Failed to kill child process (may have already exited): {}", e);
            }
            if let Err(e) = child.try_wait() {
                debug!("Failed to collect child exit status: {}", e);
            }
        }

        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.read_rx.lock().await.close();
    }

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

    fn writer_loop(mut writer: Box<dyn Write + Send>, mut write_rx: mpsc::Receiver<Vec<u8>>) {
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
        if let Ok(mut child) = self.child.lock() {
            if let Err(e) = child.kill() {
                debug!("Failed to kill child on drop (may have already exited): {}", e);
            }
            if let Err(e) = child.try_wait() {
                debug!("Failed to collect child exit status on drop: {}", e);
            }
        }

        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);

        if let Some(ref handle) = self.reader_thread {
            if !handle.is_finished() {
                debug!("PTY reader thread still running on drop, will terminate on close");
            }
        }
        if let Some(ref handle) = self.writer_thread {
            if !handle.is_finished() {
                debug!("PTY writer thread still running on drop, will terminate on close");
            }
        }
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Duration;

    #[test]
    fn test_spawn_echo_and_read_output() {
        let session = PtySession::spawn(
            &["echo".to_string(), "hello".to_string()],
            TermSize::default(),
            None,
        )
        .expect("Failed to spawn echo");

        let mut reader = session.reader().expect("Failed to get reader");

        let mut output = vec![0u8; 1024];
        let mut total_read = 0;
        std::thread::sleep(Duration::from_millis(100));

        loop {
            match reader.read(&mut output[total_read..]) {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    if total_read >= output.len() {
                        break;
                    }
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
        let session = PtySession::spawn(&["cat".to_string()], TermSize::default(), None)
            .expect("Failed to spawn cat");

        let mut writer = session.writer().expect("Failed to get writer");
        let mut reader = session.reader().expect("Failed to get reader");

        writer.write_all(b"test input\n").expect("Failed to write");
        writer.flush().expect("Failed to flush");
        std::thread::sleep(Duration::from_millis(100));

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
        let session = PtySession::spawn(&["bash".to_string()], TermSize::default(), None)
            .expect("Failed to spawn bash");

        let handle = AsyncPtyHandle::new(session).expect("Failed to create async handle");

        tokio::time::sleep(Duration::from_millis(200)).await;

        while let Ok(Some(_)) =
            tokio::time::timeout(Duration::from_millis(100), handle.read()).await
        {}

        handle.write(b"exit\n").await.expect("Failed to write exit");
        tokio::time::sleep(Duration::from_millis(200)).await;

        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while handle.read().await.is_some() {}
        })
        .await;

        let shutdown_result = tokio::time::timeout(Duration::from_secs(2), handle.shutdown()).await;
        assert!(
            shutdown_result.is_ok(),
            "Shutdown timed out, tasks may be stuck"
        );
    }

    #[tokio::test]
    async fn test_async_pty_handle_resize() {
        let session = PtySession::spawn(&["sh".to_string()], TermSize { cols: 80, rows: 24 }, None)
            .expect("spawn");

        let handle = AsyncPtyHandle::new(session).expect("async handle");

        handle
            .resize(TermSize {
                cols: 120,
                rows: 40,
            })
            .expect("resize via async handle should succeed");

        handle
            .resize(TermSize { cols: 40, rows: 10 })
            .expect("resize to smaller should succeed");
    }

    #[test]
    fn test_spawn_with_cwd() {
        let session = PtySession::spawn(&["pwd".to_string()], TermSize::default(), Some("/tmp"))
            .expect("Failed to spawn pwd with cwd");

        let mut reader = session.reader().expect("Failed to get reader");
        std::thread::sleep(Duration::from_millis(100));

        let mut output = vec![0u8; 256];
        let mut total_read = 0;

        loop {
            match reader.read(&mut output[total_read..]) {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    let s = String::from_utf8_lossy(&output[..total_read]);
                    if s.contains("tmp") {
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
            output_str.contains("tmp"),
            "Expected pwd output to contain 'tmp', got: {:?}",
            output_str
        );
    }
}
