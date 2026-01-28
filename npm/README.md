<p align="center">
  <img src="https://raw.githubusercontent.com/msmps/pilotty/main/assets/pilotty.png" alt="pilotty - Terminal automation CLI enabling AI agents to control TUI applications" width="400">
</p>

<h1 align="center">pilotty</h1>

<p align="center">
  <sub>The terminal equivalent of <a href="https://github.com/vercel-labs/agent-browser">agent-browser</a></sub>
</p>

<p align="center">
  <strong>Terminal automation CLI for AI agents</strong><br>
  <em>Control vim, htop, lazygit, dialog, and any TUI programmatically</em>
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/pilotty"><img alt="npm version" src="https://img.shields.io/npm/v/pilotty"></a>
  <a href="https://github.com/msmps/pilotty/blob/main/LICENSE"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue"></a>
</p>

---

pilotty enables AI agents to interact with terminal applications through a simple command-line interface. It manages pseudo-terminal (PTY) sessions with full VT100 terminal emulation, captures screen state, and provides keyboard/mouse input for navigating terminal user interfaces.

## Installation

```bash
npm install -g pilotty
```

## Quick Start

```bash
# Spawn a TUI application
pilotty spawn htop

# Spawn in a specific working directory
pilotty spawn --cwd /path/to/project bun src/app.tsx

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

## Documentation

See the **[GitHub repository](https://github.com/msmps/pilotty)** for full documentation including:

- All commands reference
- Session management
- Key combinations
- UI element detection
- AI agent workflow examples
- Daemon architecture

## Building from Source

```bash
git clone https://github.com/msmps/pilotty
cd pilotty
cargo build --release
./target/release/pilotty --help
```

Requires [Rust](https://rustup.rs) 1.70+.

## License

MIT
