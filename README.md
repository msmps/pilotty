<p align="center">
  <img src="assets/pilotty.png" alt="pilotty logo" width="400">
</p>

<h1 align="center">pilotty</h1>

<p align="center">
  <strong>Terminal automation CLI for AI agents</strong><br>
  <em>Like <a href="https://github.com/vercel-labs/agent-browser">agent-browser</a>, but for TUI applications.</em>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#commands">Commands</a>
</p>

---

> [!NOTE]
> **Built with AI, for AI.** This project was built with the support of an AI agent, planned thoroughly with a tight feedback loop and reviewed at each step. While we've tested extensively (153 tests!), edge cases may exist. Use in production at your own discretion, and please [report any issues](https://github.com/msmps/pilotty/issues) you find!

pilotty enables AI agents to interact with terminal applications (vim, htop, lazygit, dialog, etc.) through a simple CLI interface. It manages PTY sessions, parses terminal output, detects interactive UI elements, and provides stable references for clicking buttons, checkboxes, and menu items.

## Features

- **PTY Management**: Spawn and manage terminal applications in background sessions
- **Region Detection**: Automatically detect buttons, checkboxes, menu items, dialog boxes
- **Stable Refs**: Interactive elements get stable `@e1`, `@e2` references that persist across snapshots
- **AI-Friendly Output**: Clean JSON responses with actionable suggestions on errors
- **Multi-Session**: Run multiple terminal apps simultaneously in isolated sessions
- **Zero Config**: Daemon auto-starts on first command, auto-stops after 5 minutes idle

## Installation

### npm (recommended)

```bash
npm install -g pilotty
```

### From Source

```bash
git clone https://github.com/msmps/pilotty
cd pilotty
cargo build --release
./target/release/pilotty --help
```

Requires [Rust](https://rustup.rs) 1.70+.

## Platform Support

| Platform | Architecture | Status |
|----------|--------------|--------|
| macOS | x64 (Intel) | ✅ |
| macOS | arm64 (Apple Silicon) | ✅ |
| Linux | x64 | ✅ |
| Linux | arm64 | ✅ |
| Windows | - | ❌ Not supported |

Windows is not supported due to the use of Unix domain sockets and POSIX PTY APIs.

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

## Commands

### Session Management

```bash
pilotty spawn <command>           # Spawn a TUI app (e.g., pilotty spawn vim file.txt)
pilotty spawn <cmd> --name myapp  # Spawn with a custom session name
pilotty kill                      # Kill default session
pilotty kill -s myapp             # Kill specific session
pilotty list-sessions             # List all active sessions
pilotty stop                      # Stop the daemon and all sessions
pilotty daemon                    # Manually start daemon (usually auto-starts)
pilotty examples                  # Show end-to-end workflow example
```

### Screen Capture

```bash
pilotty snapshot                  # Full JSON with regions and text
pilotty snapshot --format compact # JSON without text field
pilotty snapshot --format text    # Plain text with cursor indicator
```

### Input

```bash
pilotty type "hello"              # Type text at cursor
pilotty key Enter                 # Send Enter key
pilotty key Ctrl+C                # Send Ctrl+C
pilotty key Alt+F                 # Send Alt+F
pilotty key F1                    # Send function key
pilotty key Tab                   # Send Tab
pilotty key Escape                # Send Escape
```

### Interaction

```bash
pilotty click @e1                 # Click region by ref
pilotty scroll up                 # Scroll up 1 line
pilotty scroll down 5             # Scroll down 5 lines
```

### Terminal Control

```bash
pilotty resize 120 40             # Resize terminal to 120x40
pilotty wait-for "Ready"          # Wait for text to appear
pilotty wait-for "Error" --regex  # Wait for regex pattern
pilotty wait-for "Done" -t 5000   # Wait with 5s timeout
```

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
    },
    {
      "ref_id": "@e2",
      "bounds": { "x": 20, "y": 5, "width": 10, "height": 1 },
      "region_type": "button",
      "text": "[ Cancel ]",
      "focused": false
    }
  ],
  "text": "... plain text content ..."
}
```

### Region Types

pilotty automatically detects:

| Type | Example | Detection |
|------|---------|-----------|
| `button` | `[ OK ]`, `< Save >` | Bracketed text with padding |
| `checkbox` | `[x] Enable`, `[ ] Disable` | Square brackets with x or space |
| `radio_button` | `(*) Option`, `( ) Other` | Parentheses with * or space |
| `menu_item` | `(F)ile`, highlighted text | Shortcut pattern or inverse video |
| `link` | Underlined text | Underline attribute |
| `text_input` | Dialog input fields | Box-drawing characters |
| `scrollable_area` | Scroll regions | Detected contextually |
| `unknown` | Unclassified boxes | Box detected but no pattern match |

### Stable Refs

Region refs (`@e1`, `@e2`, etc.) are stable across snapshots when the content and position are similar. This allows agents to:

1. Take a snapshot, identify `@e1` as the OK button
2. Perform other operations
3. Click `@e1` without re-scanning

```bash
pilotty snapshot           # "@e1" is [ OK ]
pilotty type "some text"
pilotty click @e1          # Still works!
```

## Sessions

Each session is an isolated terminal with its own:
- PTY (pseudo-terminal)
- Screen buffer
- Region tracker
- Child process

```bash
# Run multiple apps
pilotty spawn htop --name monitoring
pilotty spawn vim file.txt --name editor

# Target specific session
pilotty snapshot -s monitoring
pilotty key -s editor Ctrl+S

# List all sessions
pilotty list-sessions
```

If no `--session` is specified, pilotty uses the default session.

Note: The first session spawned without `--name` is automatically named `default`.
To run multiple sessions, give each a unique name with `--name`:

```bash
pilotty spawn htop --name monitoring
pilotty spawn vim --name editor
```

## Daemon Architecture

pilotty uses a daemon architecture similar to agent-browser:

```
┌─────────────┐     Unix Socket      ┌─────────────────┐
│   CLI       │ ──────────────────▶  │     Daemon      │
│  (pilotty)  │     JSON-line        │  (auto-started) │
└─────────────┘                      └─────────────────┘
                                              │
                                     ┌────────┴────────┐
                                     ▼                 ▼
                              ┌───────────┐     ┌───────────┐
                              │  Session  │     │  Session  │
                              │  (htop)   │     │  (vim)    │
                              └───────────┘     └───────────┘
```

- **Auto-start**: Daemon starts automatically on first command
- **Auto-stop**: Daemon shuts down after 5 minutes with no active sessions
- **Session cleanup**: Sessions are automatically removed when their process exits
- **Background**: Runs in background, survives terminal close
- **Shared state**: Multiple CLI invocations share sessions
- **Clean shutdown**: `pilotty stop` gracefully terminates all sessions

### Lifecycle

The daemon is designed for zero-maintenance operation:

1. **First command** (e.g., `pilotty spawn vim`) starts the daemon automatically
2. **Session ends** (e.g., vim exits after `:wq`) and the session is cleaned up within 500ms
3. **Idle timeout**: After 5 minutes with no sessions, the daemon shuts down
4. **Next command** starts the daemon again automatically

This means you never need to manually manage the daemon, it starts when needed and stops when idle.

### Socket Location

The daemon socket is created at (in priority order):
1. `$PILOTTY_SOCKET_DIR/{session}.sock` (explicit override)
2. `$XDG_RUNTIME_DIR/pilotty/{session}.sock` (Linux standard)
3. `~/.pilotty/{session}.sock` (home directory fallback)
4. `/tmp/pilotty/{session}.sock` (last resort)

## Error Handling

All errors include AI-friendly suggestions:

```json
{
  "code": "SESSION_NOT_FOUND",
  "message": "Session 'abc123' not found",
  "suggestion": "Run 'pilotty list-sessions' to see available sessions"
}
```

```json
{
  "code": "REF_NOT_FOUND",
  "message": "Region '@e5' not found",
  "suggestion": "Run 'pilotty snapshot' to get updated refs. Available: @e1 ([ OK ]), @e2 ([ Cancel ])"
}
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PILOTTY_SESSION` | Default session name |
| `PILOTTY_SOCKET_DIR` | Override socket directory |
| `RUST_LOG` | Logging level (e.g., `debug`, `info`) |

## AI Agent Workflow

Recommended workflow for AI agents:

```bash
# 1. Spawn the application
pilotty spawn vim myfile.txt

# 2. Wait for it to be ready
pilotty wait-for "myfile.txt"

# 3. Take a snapshot to understand the screen
pilotty snapshot --format full

# 4. Parse the JSON, identify interactive elements
# 5. Perform actions using refs
pilotty key i                    # Enter insert mode
pilotty type "Hello, World!"
pilotty key Escape
pilotty type ":wq"
pilotty key Enter

# 6. Take another snapshot if needed
pilotty snapshot
```

## Key Combinations

Supported key formats:

| Format | Example | Notes |
|--------|---------|-------|
| Named keys | `Enter`, `Tab`, `Escape`, `Space`, `Backspace` | Case insensitive |
| Arrow keys | `Up`, `Down`, `Left`, `Right` | Also: `ArrowUp`, etc. |
| Navigation | `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete` | Also: `PgUp`, `PgDn`, `Ins`, `Del` |
| Function keys | `F1` - `F12` | |
| Ctrl combos | `Ctrl+C`, `Ctrl+X`, `Ctrl+Z` | Also: `Control+C` |
| Alt combos | `Alt+F`, `Alt+X` | Also: `Meta+F`, `Option+F` |
| Shift combos | `Shift+A` | Only uppercases letter keys |
| Combined | `Ctrl+Alt+C` | |
| Special | `Plus` | Literal `+` character |
| Aliases | `Return` = `Enter`, `Esc` = `Escape` | |

## Contributing

Contributions welcome! Please:

1. Run `cargo fmt` before committing
2. Run `cargo clippy --all --all-features` and fix warnings
3. Add tests for new functionality
4. Update documentation as needed

## License

MIT

## Acknowledgments

- Inspired by [agent-browser](https://github.com/vercel-labs/agent-browser) by Vercel Labs
- Built with [vt100](https://crates.io/crates/vt100) for terminal emulation
- Built with [portable-pty](https://crates.io/crates/portable-pty) for PTY management
