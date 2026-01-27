<p align="center">
  <img src="https://raw.githubusercontent.com/msmps/pilotty/main/assets/pilotty.png" alt="pilotty logo" width="400">
</p>

<h1 align="center">pilotty</h1>

<p align="center">
  <strong>Terminal automation CLI for AI agents</strong><br>
  <em>Like <a href="https://github.com/vercel-labs/agent-browser">agent-browser</a>, but for TUI applications.</em>
</p>

---

pilotty enables AI agents to interact with terminal applications (vim, htop, lazygit, dialog, etc.) through a simple CLI interface. It manages PTY sessions, parses terminal output, detects interactive UI elements, and provides stable references for clicking buttons, checkboxes, and menu items.

## Features

- **PTY Management**: Spawn and manage terminal applications in background sessions
- **Region Detection**: Automatically detect buttons, checkboxes, menu items, dialog boxes
- **Stable Refs**: Interactive elements get stable `@e1`, `@e2` references that persist across snapshots
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

# Click an interactive region by ref
pilotty click @e1

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
  "regions": [
    {
      "ref_id": "@e1",
      "bounds": { "x": 10, "y": 5, "width": 6, "height": 1 },
      "region_type": "button",
      "text": "[ OK ]",
      "focused": false
    }
  ],
  "text": "... plain text content ..."
}
```

## Building from Source

```bash
git clone https://github.com/msmps/pilotty
cd pilotty
cargo build --release
./target/release/pilotty --help
```

Requires [Rust](https://rustup.rs) 1.70+.

## Documentation

See the [GitHub repository](https://github.com/msmps/pilotty) for full documentation including all commands, region types, key combinations, and AI agent workflow examples.

## License

MIT
