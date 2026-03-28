//! pilotty CLI and daemon entry point.

mod args;
mod daemon;

use std::path::Path;

use anyhow::bail;
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
fn cli_to_command(cli: &Cli) -> anyhow::Result<Option<Command>> {
    match &cli.command {
        Commands::Spawn(args) => {
            if args.shell_program.is_some() && !args.shell {
                bail!("--shell-program requires --shell");
            }

            let command = if args.shell {
                build_shell_command(&args.command, args.shell_program.as_deref())?
            } else {
                args.command.clone()
            };

            Ok(Some(Command::Spawn {
                command,
                session_name: args.name.clone(),
                // Default to client's cwd if --cwd not explicitly provided
                cwd: args.cwd.clone().or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().into_owned())
                }),
            }))
        }
        Commands::Kill(args) => Ok(Some(Command::Kill {
            session: args.session.clone(),
        })),
        Commands::Snapshot(args) => Ok(Some(Command::Snapshot {
            session: args.session.clone(),
            format: Some(match args.format {
                crate::args::SnapshotFormat::Full => SnapshotFormat::Full,
                crate::args::SnapshotFormat::Compact => SnapshotFormat::Compact,
                crate::args::SnapshotFormat::Text => SnapshotFormat::Text,
            }),
            await_change: args.await_change,
            settle_ms: args.settle,
            timeout_ms: args.timeout,
        })),
        Commands::Type(args) => Ok(Some(Command::Type {
            text: args.text.clone(),
            session: args.session.clone(),
        })),
        Commands::Key(args) => Ok(Some(Command::Key {
            key: args.key.clone(),
            delay_ms: args.delay,
            session: args.session.clone(),
        })),
        Commands::Click(args) => Ok(Some(Command::Click {
            row: args.row,
            col: args.col,
            session: args.session.clone(),
        })),
        Commands::Scroll(args) => Ok(Some(Command::Scroll {
            direction: match args.direction {
                crate::args::ScrollDirection::Up => ScrollDirection::Up,
                crate::args::ScrollDirection::Down => ScrollDirection::Down,
            },
            amount: args.amount,
            session: args.session.clone(),
        })),
        Commands::ListSessions => Ok(Some(Command::ListSessions)),
        Commands::Resize(args) => Ok(Some(Command::Resize {
            cols: args.cols,
            rows: args.rows,
            session: args.session.clone(),
        })),
        Commands::WaitFor(args) => Ok(Some(Command::WaitFor {
            pattern: args.pattern.clone(),
            timeout_ms: Some(args.timeout),
            regex: Some(args.regex),
            session: args.session.clone(),
        })),
        Commands::Daemon => unreachable!("Daemon command handled separately"),
        Commands::Examples => Ok(None),
        Commands::Stop => Ok(Some(Command::Shutdown)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

fn build_shell_command(command: &[String], shell_program_override: Option<&str>) -> anyhow::Result<Vec<String>> {
    if command.is_empty() {
        bail!("Shell command cannot be empty");
    }

    let shell_program = resolve_shell_program(shell_program_override)?;
    let shell_kind = classify_shell(&shell_program);
    let shell_command = match shell_kind {
        ShellKind::Posix => quote_posix_command(command),
        ShellKind::PowerShell => quote_powershell_command(command),
        ShellKind::Cmd => quote_cmd_command(command),
    };

    let args = match shell_kind {
        ShellKind::Posix => vec!["-lc".to_string(), shell_command],
        ShellKind::PowerShell => vec!["-NoLogo".to_string(), "-Command".to_string(), shell_command],
        ShellKind::Cmd => vec!["/d".to_string(), "/s".to_string(), "/c".to_string(), shell_command],
    };

    let mut result = Vec::with_capacity(1 + args.len());
    result.push(shell_program);
    result.extend(args);
    Ok(result)
}

fn resolve_shell_program(shell_program_override: Option<&str>) -> anyhow::Result<String> {
    if let Some(program) = shell_program_override.filter(|s| !s.trim().is_empty()) {
        return Ok(program.to_string());
    }

    for candidate in [
        std::env::var("PILOTTY_SHELL").ok(),
        std::env::var("SHELL").ok(),
        #[cfg(windows)]
        std::env::var("COMSPEC").ok(),
    ]
    .into_iter()
    .flatten()
    {
        if !candidate.trim().is_empty() {
            return Ok(candidate);
        }
    }

    #[cfg(windows)]
    {
        let git_bash = "C:/Program Files/Git/bin/bash.exe";
        if Path::new(git_bash).exists() {
            return Ok(git_bash.to_string());
        }
        return Ok("powershell.exe".to_string());
    }

    #[cfg(not(windows))]
    {
        Ok("/bin/sh".to_string())
    }
}

fn classify_shell(program: &str) -> ShellKind {
    let lower = program.replace('\\', "/").to_ascii_lowercase();
    if lower.ends_with("cmd") || lower.ends_with("cmd.exe") || lower == "cmd" {
        ShellKind::Cmd
    } else if lower.contains("powershell") || lower.ends_with("pwsh") || lower.ends_with("pwsh.exe") {
        ShellKind::PowerShell
    } else {
        ShellKind::Posix
    }
}

fn quote_posix_command(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            if arg.is_empty() {
                "''".to_string()
            } else {
                format!("'{}'", arg.replace('\'', "'\"'\"'"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_powershell_command(command: &[String]) -> String {
    let mut parts = command.iter();
    let Some(program) = parts.next() else {
        return String::new();
    };

    let mut rendered = format!("& '{}'", program.replace('\'', "''"));
    for arg in parts {
        rendered.push(' ');
        rendered.push_str(&format!("'{}'", arg.replace('\'', "''")));
    }
    rendered
}

fn quote_cmd_command(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            if arg.is_empty() {
                "\"\"".to_string()
            } else if arg.chars().any(|c| c.is_whitespace() || matches!(c, '&' | '|' | '<' | '>' | '^' | '"')) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run a client command by connecting to the daemon.
fn run_client_command(cli: Cli) -> anyhow::Result<()> {
    // Handle commands that don't need daemon communication
    let Some(command) = cli_to_command(&cli)? else {
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
            _ = ctrl_c_signal() => {
                info!("Received SIGINT, shutting down gracefully");
            }
            _ = sigterm() => {
                info!("Received SIGTERM, shutting down gracefully");
            }
        }
        // Server is dropped here, triggering cleanup of socket and PID files
    });
}

/// Wait for SIGINT / Ctrl+C.
///
/// On Windows the auto-started daemon runs detached without a console, so listening
/// for Ctrl+C can cause premature shutdown. Detached daemons are stopped via the
/// explicit `pilotty stop` command instead.
#[cfg(unix)]
async fn ctrl_c_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::warn!("Failed to register SIGINT handler: {}", e);
        std::future::pending::<()>().await;
    }
}

#[cfg(windows)]
async fn ctrl_c_signal() {
    std::future::pending::<()>().await;
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
