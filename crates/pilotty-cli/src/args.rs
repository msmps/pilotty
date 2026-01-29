//! CLI argument parsing with clap derive macros.

use clap::{Parser, Subcommand, ValueEnum};

const SESSION_HELP: &str = "Target session by name or ID [default: default]";

/// Terminal automation for AI agents.
///
/// Spawn TUI applications in managed PTY sessions and interact with them
/// programmatically. Designed for AI agent consumption with structured
/// JSON output and stable element references.
#[derive(Debug, Parser)]
#[command(name = "pilotty", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Spawn a new TUI application in a managed PTY session
    #[command(after_help = "\
Examples:
  pilotty spawn htop                    # Simple command
  pilotty spawn vim file.txt            # Command with arguments
  pilotty spawn --name editor vim       # Named session for easy reference
  pilotty spawn --cwd /tmp bash         # Start bash in /tmp directory
  pilotty spawn bash -c 'echo hello'    # Shell command with args")]
    Spawn(SpawnArgs),

    /// Kill a session and its child process
    Kill(KillArgs),

    /// Get a snapshot of the terminal screen
    #[command(after_help = "\
Examples:
  pilotty snapshot                      # Snapshot default session (full JSON)
  pilotty snapshot --format compact     # JSON without text field
  pilotty snapshot --format text        # Plain text with cursor indicator
  pilotty snapshot -s editor            # Snapshot a specific session

Wait for change:
  HASH=$(pilotty snapshot | jq -r '.content_hash')
  pilotty key Enter
  pilotty snapshot --await-change $HASH           # Block until screen changes
  pilotty snapshot --await-change $HASH --settle 100  # Wait for 100ms stability")]
    Snapshot(SnapshotArgs),

    /// Type text at the current cursor position
    #[command(
        name = "type",
        after_help = "\
Examples:
  pilotty type 'Hello, world!'          # Type literal text
  pilotty type \"line1\\nline2\"          # Type with newline (shell escaping)
  pilotty type -s editor ':wq'          # Type in a specific session"
    )]
    Type(TypeArgs),

    /// Send a key, key combination, or key sequence
    #[command(after_long_help = "\
Supported Keys:
  Navigation:  Enter, Tab, Escape, Backspace, Space, Delete, Insert
  Arrows:      Up, Down, Left, Right, Home, End, PageUp, PageDown
  Function:    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12
  Modifiers:   Ctrl+<key>, Alt+<key>

Key Sequences:
  Space-separated keys are sent in order. Useful for chords like Emacs C-x m.

Examples:
  pilotty key Enter                     # Press enter
  pilotty key Ctrl+C                    # Send interrupt signal
  pilotty key Alt+F                     # Alt+F (often opens File menu)
  pilotty key \"Ctrl+X m\"                # Emacs chord: Ctrl+X then m
  pilotty key \"Escape : w q Enter\"      # vim :wq sequence
  pilotty key \"Ctrl+X Ctrl+S\"           # Emacs save (two combos)
  pilotty key -s editor Escape          # Send Escape to specific session
  pilotty key \"a b c\" --delay 50        # Send a, b, c with 50ms delay between")]
    Key(KeyArgs),

    /// Click at a specific row and column coordinate
    #[command(after_help = "\
Click at a specific position in the terminal using 0-indexed coordinates.
Use 'pilotty snapshot' to see cursor position and terminal dimensions.

Examples:
  pilotty click 10 5                    # Click at row 10, column 5
  pilotty click -s editor 5 20          # Click in a specific session")]
    Click(ClickArgs),

    /// Scroll the terminal up or down
    Scroll(ScrollArgs),

    /// List all active sessions
    ListSessions,

    /// Resize the terminal
    Resize(ResizeArgs),

    /// Wait for text to appear on screen
    #[command(after_help = "\
Examples:
  pilotty wait-for 'Ready'              # Wait for literal text
  pilotty wait-for -r 'error|warning'   # Wait for regex pattern
  pilotty wait-for -t 5000 'Done'       # Wait up to 5 seconds
  pilotty wait-for -s editor '~'        # Wait in specific session")]
    WaitFor(WaitForArgs),

    /// Show an end-to-end usage example
    Examples,

    /// Start the daemon process (usually auto-started)
    Daemon,

    /// Stop the daemon process
    Stop,
}

#[derive(Debug, clap::Args)]
pub struct SpawnArgs {
    /// Command and arguments to run (e.g., vim, htop, bash)
    #[arg(
        required = true,
        num_args = 1..,
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub command: Vec<String>,

    /// Give this session a human-readable name
    #[arg(short, long)]
    pub name: Option<String>,

    /// Working directory for the spawned process.
    /// If omitted, inherits the daemon's current directory.
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct KillArgs {
    /// Target session by name or ID [default: default]
    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct SnapshotArgs {
    /// Output format
    #[arg(short, long, value_enum, default_value_t = SnapshotFormat::Full)]
    pub format: SnapshotFormat,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,

    /// Block until content_hash differs from this value
    #[arg(long, value_name = "HASH")]
    pub await_change: Option<u64>,

    /// Wait for screen to stabilize for this many ms before returning
    #[arg(long, default_value_t = 0, value_name = "MS")]
    pub settle: u64,

    /// Timeout in milliseconds for await-change/settle (default: 30s)
    #[arg(short, long, default_value_t = 30000)]
    pub timeout: u64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SnapshotFormat {
    /// Full JSON with all metadata
    Full,
    /// Compact format with inline refs
    Compact,
    /// Plain text only
    Text,
}

#[derive(Debug, clap::Args)]
pub struct TypeArgs {
    /// Text to type
    pub text: String,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct KeyArgs {
    /// Key, combo, or sequence to send (e.g., Enter, Ctrl+C, "Ctrl+X m")
    pub key: String,

    /// Delay between keys in a sequence (milliseconds, max 10000)
    #[arg(long, default_value_t = 0)]
    pub delay: u32,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ClickArgs {
    /// Row coordinate (0-indexed)
    pub row: u16,

    /// Column coordinate (0-indexed)
    pub col: u16,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ScrollArgs {
    /// Direction to scroll
    #[arg(value_enum)]
    pub direction: ScrollDirection,

    /// Number of lines to scroll
    #[arg(default_value_t = 1)]
    pub amount: u32,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug, clap::Args)]
pub struct ResizeArgs {
    /// Number of columns
    pub cols: u16,

    /// Number of rows
    pub rows: u16,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct WaitForArgs {
    /// Text or regex pattern to wait for
    pub pattern: String,

    /// Timeout in milliseconds
    #[arg(short, long, default_value_t = 30000)]
    pub timeout: u64,

    /// Treat pattern as regex
    #[arg(short, long)]
    pub regex: bool,

    #[arg(short, long, help = SESSION_HELP)]
    pub session: Option<String>,
}

/// End-to-end example text for the `examples` command.
pub const EXAMPLES_TEXT: &str = r#"End-to-end example: Create a file with vi

This example spawns vi, writes text to a file, saves, and exits.

# 1. Spawn vi to create a new file
pilotty spawn --name editor vi /tmp/hello.txt

# 2. Wait for vi to start
pilotty wait-for -s editor "hello.txt"

# 3. Press 'i' to enter insert mode
pilotty key -s editor i

# 4. Type some text
pilotty type -s editor "Hello from pilotty!"

# 5. Press Escape to return to normal mode
pilotty key -s editor Escape

# 6. Save and quit with :wq
pilotty type -s editor ":wq"
pilotty key -s editor Enter

# 7. Verify the session ended (vi exited)
pilotty list-sessions

# The file /tmp/hello.txt now contains "Hello from pilotty!"
"#;

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn test_spawn_parses_hyphenated_args() {
        let cli = Cli::parse_from(["pilotty", "spawn", "bash", "-c", "echo hello"]);

        match cli.command {
            Commands::Spawn(args) => {
                assert_eq!(args.command, vec!["bash", "-c", "echo hello"]);
            }
            _ => panic!("Expected spawn command"),
        }
    }
}
