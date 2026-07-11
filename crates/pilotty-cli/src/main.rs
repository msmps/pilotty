//! pilotty CLI and daemon entry point.

mod args;
mod daemon;

use clap::Parser;
use pilotty_core::error::ErrorCode;
use pilotty_core::protocol::{
    CaptureOutcome, Command, OutputFormat, Request, ResponseData, ScrollDirection, SnapshotFormat,
};
use std::io::Write;
use tracing::{error, info};
use uuid::Uuid;

use crate::args::{Cli, Commands};
use crate::daemon::client::DaemonClient;
use crate::daemon::server::DaemonServer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliExitCode {
    Success = 0,
    GenericError = 1,
    Timing = 3,
    Lifecycle = 4,
}

impl CliExitCode {
    fn value(self) -> u8 {
        self as u8
    }
}

fn main() -> std::process::ExitCode {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    // Daemon command runs the server, all other commands are clients
    if let Commands::Daemon = cli.command {
        run_daemon();
        return std::process::ExitCode::SUCCESS;
    }

    // All other commands talk to the daemon
    match run_client_command(cli) {
        Ok(CliExitCode::Success) => {}
        Ok(code) => {
            if let Err(error) = std::io::stdout().flush() {
                error!("Failed to flush command evidence: {}", error);
            }
            return std::process::ExitCode::from(code.value());
        }
        Err(e) => {
            error!("{}", e);
            return std::process::ExitCode::from(CliExitCode::GenericError.value());
        }
    }

    std::process::ExitCode::SUCCESS
}

/// Convert CLI args to a protocol Command.
///
/// Returns None for commands that don't require daemon communication.
fn cli_to_command(cli: &Cli) -> Option<Command> {
    match &cli.command {
        Commands::Spawn(args) => Some(Command::Spawn {
            command: args.command.clone(),
            session_name: args.name.clone(),
            // Default to client's cwd if --cwd not explicitly provided
            cwd: args.cwd.clone().or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            }),
            retain_bytes: args.retain_bytes,
        }),
        Commands::Kill(args) => Some(Command::Kill {
            session: args.session.clone(),
        }),
        Commands::Snapshot(args) => Some(Command::Snapshot {
            session: args.session.clone(),
            format: match args.format {
                crate::args::SnapshotFormat::Full => SnapshotFormat::Full,
                crate::args::SnapshotFormat::Compact => SnapshotFormat::Compact,
                crate::args::SnapshotFormat::Text => SnapshotFormat::Text,
            },
            await_change: args.await_change,
            settle_ms: args.settle,
            timeout_ms: args.timeout,
        }),
        Commands::Type(args) => Some(Command::Type {
            text: args.text.clone(),
            session: args.session.clone(),
        }),
        Commands::Key(args) => Some(Command::Key {
            key: args.key.clone(),
            delay_ms: args.delay,
            session: args.session.clone(),
        }),
        Commands::Click(args) => Some(Command::Click {
            row: args.row,
            col: args.col,
            session: args.session.clone(),
        }),
        Commands::Scroll(args) => Some(Command::Scroll {
            direction: match args.direction {
                crate::args::ScrollDirection::Up => ScrollDirection::Up,
                crate::args::ScrollDirection::Down => ScrollDirection::Down,
            },
            amount: args.amount,
            session: args.session.clone(),
        }),
        Commands::ListSessions => Some(Command::ListSessions),
        Commands::Output(args) => Some(Command::Output {
            session: args.session.clone(),
            ansi: args.ansi,
        }),
        Commands::Status(args) => Some(Command::Status {
            session: args.session.clone(),
        }),
        Commands::Resize(args) => Some(Command::Resize {
            cols: args.cols,
            rows: args.rows,
            session: args.session.clone(),
        }),
        Commands::WaitFor(args) => Some(Command::WaitFor {
            pattern: args.pattern.clone(),
            timeout_ms: Some(args.timeout),
            regex: Some(args.regex),
            session: args.session.clone(),
        }),
        Commands::Daemon => unreachable!("Daemon command handled separately"),
        Commands::Examples => None,
        Commands::Stop => Some(Command::Shutdown),
    }
}

/// Run a client command by connecting to the daemon.
fn run_client_command(cli: Cli) -> anyhow::Result<CliExitCode> {
    let strict = matches!(
        &cli.command,
        Commands::Snapshot(args) if args.strict
    );
    let targets_live_session = matches!(
        &cli.command,
        Commands::Type(_)
            | Commands::Key(_)
            | Commands::Click(_)
            | Commands::Scroll(_)
            | Commands::Resize(_)
    );

    // Handle commands that don't need daemon communication
    let Some(command) = cli_to_command(&cli) else {
        // Examples command just prints and exits
        if let Commands::Examples = cli.command {
            println!("{}", crate::args::EXAMPLES_TEXT);
        }
        return Ok(CliExitCode::Success);
    };

    let runtime = tokio::runtime::Runtime::new()?;

    runtime.block_on(async {
        // Connect to daemon (auto-starts if not running)
        let mut client = DaemonClient::connect().await?;

        // Build request
        let request = Request::new(Uuid::new_v4().to_string(), command);

        // Send request and get response
        let response = client.request(request).await?;

        // Print response
        let exit_code = if response.success {
            if let Some(data) = response.data {
                let outcome = data.capture_outcome();
                match data {
                    ResponseData::Snapshot {
                        format: SnapshotFormat::Text,
                        content,
                        ..
                    } => {
                        println!("{}", content);
                    }
                    ResponseData::Output {
                        format,
                        bytes,
                        total_bytes,
                        retained_bytes,
                        dropped_bytes,
                        truncated,
                    } => {
                        std::io::stdout().write_all(&bytes)?;
                        if format == OutputFormat::Text && !bytes.ends_with(b"\n") {
                            std::io::stdout().write_all(b"\n")?;
                        }
                        std::io::stdout().flush()?;
                        let format_name = match format {
                            OutputFormat::Text => "text",
                            OutputFormat::Ansi => "ansi",
                        };
                        eprintln!(
                            "retention: format={format_name} total_bytes={total_bytes} \
                             retained_bytes={retained_bytes} dropped_bytes={dropped_bytes} \
                             truncated={truncated}"
                        );
                    }
                    _ => println!("{}", serde_json::to_string_pretty(&data)?),
                }

                capture_exit_code(strict, outcome)
            } else {
                CliExitCode::Success
            }
        } else if let Some(err) = response.error {
            eprintln!("Error: {}", err);
            api_error_exit_code(targets_live_session, &err.code)
        } else {
            CliExitCode::GenericError
        };

        Ok(exit_code)
    })
}

fn capture_exit_code(strict: bool, outcome: Option<CaptureOutcome>) -> CliExitCode {
    if !strict {
        return CliExitCode::Success;
    }

    match outcome {
        Some(CaptureOutcome::Deadline) => CliExitCode::Timing,
        Some(CaptureOutcome::Exited) => CliExitCode::Lifecycle,
        Some(CaptureOutcome::Immediate | CaptureOutcome::Settled | CaptureOutcome::Changed)
        | None => CliExitCode::Success,
    }
}

fn api_error_exit_code(targets_live_session: bool, error: &ErrorCode) -> CliExitCode {
    if targets_live_session && *error == ErrorCode::SessionExited {
        CliExitCode::Lifecycle
    } else {
        CliExitCode::GenericError
    }
}

/// Run the daemon server with graceful signal handling.
///
/// Handles SIGINT (Ctrl+C) and SIGTERM for clean shutdown.
/// The DaemonServer's Drop impl cleans up socket and PID files.
fn run_daemon() {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            error!("Failed to create tokio runtime: {}", e);
            std::process::exit(1);
        }
    };

    runtime.block_on(async {
        let server = match DaemonServer::bind().await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to start daemon: {}", e);
                std::process::exit(1);
            }
        };

        // Run server with signal handling
        tokio::select! {
            result = server.run() => {
                if let Err(e) = result {
                    error!("Daemon error: {}", e);
                    std::process::exit(1);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT, shutting down gracefully");
            }
            _ = sigterm() => {
                info!("Received SIGTERM, shutting down gracefully");
            }
        }
        // Server is dropped here, triggering cleanup of socket and PID files
    });
}

/// Wait for SIGTERM signal (Unix only).
///
/// If signal registration fails, logs a warning and waits indefinitely.
/// This graceful fallback prevents panics during daemon startup.
#[cfg(unix)]
async fn sigterm() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            sigterm.recv().await;
        }
        Err(e) => {
            tracing::warn!(
                "Failed to register SIGTERM handler: {}, daemon will only respond to SIGINT",
                e
            );
            std::future::pending::<()>().await;
        }
    }
}

/// SIGTERM is not available on non-Unix platforms; use a never-completing future.
#[cfg(not(unix))]
async fn sigterm() {
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use pilotty_core::error::ErrorCode;
    use pilotty_core::protocol::CaptureOutcome;

    use crate::{api_error_exit_code, capture_exit_code, CliExitCode};

    #[test]
    fn strict_capture_exit_codes_are_categorical() {
        assert_eq!(
            capture_exit_code(true, Some(CaptureOutcome::Deadline)),
            CliExitCode::Timing
        );
        assert_eq!(
            capture_exit_code(true, Some(CaptureOutcome::Exited)),
            CliExitCode::Lifecycle
        );
        assert_eq!(
            capture_exit_code(true, Some(CaptureOutcome::Settled)),
            CliExitCode::Success
        );
        assert_eq!(
            capture_exit_code(false, Some(CaptureOutcome::Deadline)),
            CliExitCode::Success
        );
    }

    #[test]
    fn exited_input_uses_lifecycle_exit_code() {
        assert_eq!(
            api_error_exit_code(true, &ErrorCode::SessionExited),
            CliExitCode::Lifecycle
        );
        assert_eq!(
            api_error_exit_code(false, &ErrorCode::SessionExited),
            CliExitCode::GenericError
        );
        assert_eq!(
            api_error_exit_code(true, &ErrorCode::InvalidInput),
            CliExitCode::GenericError
        );
    }
}
