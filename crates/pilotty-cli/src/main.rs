//! pilotty CLI and daemon entry point.

mod args;
mod daemon;

use clap::Parser;
use pilotty_core::protocol::{Command, Request, ResponseData, ScrollDirection, SnapshotFormat};
use tracing::{error, info};
use uuid::Uuid;

use crate::args::{Cli, Commands};
use crate::daemon::client::DaemonClient;
use crate::daemon::server::DaemonServer;

fn main() {
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
        return;
    }

    // All other commands talk to the daemon
    if let Err(e) = run_client_command(cli) {
        error!("{}", e);
        std::process::exit(1);
    }
}

/// Convert CLI args to a protocol Command.
///
/// Returns None for commands that don't require daemon communication.
fn cli_to_command(cli: &Cli) -> Option<Command> {
    match &cli.command {
        Commands::Spawn(args) => Some(Command::Spawn {
            command: args.command.clone(),
            session_name: args.name.clone(),
        }),
        Commands::Kill(args) => Some(Command::Kill {
            session: args.session.clone(),
        }),
        Commands::Snapshot(args) => Some(Command::Snapshot {
            session: args.session.clone(),
            format: Some(match args.format {
                crate::args::SnapshotFormat::Full => SnapshotFormat::Full,
                crate::args::SnapshotFormat::Compact => SnapshotFormat::Compact,
                crate::args::SnapshotFormat::Text => SnapshotFormat::Text,
            }),
        }),
        Commands::Type(args) => Some(Command::Type {
            text: args.text.clone(),
            session: args.session.clone(),
        }),
        Commands::Key(args) => Some(Command::Key {
            key: args.key.clone(),
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
fn run_client_command(cli: Cli) -> anyhow::Result<()> {
    // Handle commands that don't need daemon communication
    let Some(command) = cli_to_command(&cli) else {
        // Examples command just prints and exits
        if let Commands::Examples = cli.command {
            println!("{}", crate::args::EXAMPLES_TEXT);
        }
        return Ok(());
    };

    let runtime = tokio::runtime::Runtime::new()?;

    runtime.block_on(async {
        // Connect to daemon (auto-starts if not running)
        let mut client = DaemonClient::connect().await?;

        // Build request
        let request = Request {
            id: Uuid::new_v4().to_string(),
            command,
        };

        // Send request and get response
        let response = client.request(request).await?;

        // Print response
        if response.success {
            if let Some(data) = response.data {
                match data {
                    ResponseData::Snapshot {
                        format: SnapshotFormat::Text,
                        content,
                    } => {
                        println!("{}", content);
                    }
                    _ => println!("{}", serde_json::to_string_pretty(&data)?),
                }
            }
        } else if let Some(err) = response.error {
            eprintln!("Error: {}", err);
            std::process::exit(1);
        }

        Ok(())
    })
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
