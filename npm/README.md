<p align="center">
  <img src="https://raw.githubusercontent.com/msmps/pilotty/main/assets/pilotty.png" alt="pilotty logo" width="400">
</p>

<h1 align="center">pilotty</h1>

<p align="center">
  <strong>Terminal automation CLI for AI agents</strong><br>
  <em>Like <a href="https://github.com/vercel-labs/agent-browser">agent-browser</a>, but for TUI applications.</em>
</p>

---

pilotty enables AI agents to interact with terminal applications (vim, htop, lazygit, dialog, etc.) through a simple CLI interface. It manages PTY sessions, captures terminal output, and provides keyboard/mouse input capabilities for navigating TUI applications.

## Features

- **PTY Management**: Spawn and manage terminal applications in background sessions
- **Keyboard Navigation**: Interact with TUIs using Tab, Enter, arrow keys, and key combos
- **AI-Friendly Output**: Clean JSON responses with actionable suggestions on errors
- **Multi-Session**: Run multiple terminal apps simultaneously in isolated sessions
- **Zero Config**: Daemon auto-starts on first command, auto-stops after 5 minutes idle

## Installation

```bash
npm install -g pilotty
```

## Quick Start

```bash
# Spawn a TUI application
pilotty spawn htop

# Take a snapshot of the terminal
pilotty snapshot

# Type text
pilotty type "hello world"

# Send keys
pilotty key Enter
pilotty key Ctrl+C

# Click at specific coordinates (row, col)
pilotty click 10 5

# List active sessions
pilotty list-sessions

# Stop the daemon
pilotty stop
```

## Platform Support

| Platform | Architecture | Status |
|----------|--------------|--------|
| macOS | x64 (Intel) | Supported |
| macOS | arm64 (Apple Silicon) | Supported |
| Linux | x64 | Supported |
| Linux | arm64 | Supported |
| Windows | - | Not supported |

Windows is not supported due to the use of Unix domain sockets and POSIX PTY APIs.

## Snapshot Output

The `snapshot` command returns structured data about the terminal screen:

```json
{
  "snapshot_id": 42,
  "size": { "cols": 80, "rows": 24 },
  "cursor": { "row": 5, "col": 10, "visible": true },
  "text": "... plain text content ..."
}
```

Use the cursor position and text content to understand the screen state and navigate using keyboard commands (Tab, Enter, arrow keys) or click at specific coordinates.

## Building from Source

```bash
git clone https://github.com/msmps/pilotty
cd pilotty
cargo build --release
./target/release/pilotty --help
```

Requires [Rust](https://rustup.rs) 1.70+.

## Documentation

See the [GitHub repository](https://github.com/msmps/pilotty) for full documentation including all commands, key combinations, and AI agent workflow examples.

## License

MIT
