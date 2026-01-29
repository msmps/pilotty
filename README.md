<p align="center">
  <img src="assets/pilotty.png" alt="pilotty - Terminal automation CLI enabling AI agents to control TUI applications" width="400">
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
  <a href="#installation">Installation</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#commands">Commands</a> •
  <a href="#usage-with-ai-agents">AI Agents</a>
</p>

---

> [!NOTE]
> **Built with AI, for AI.** This project was built with the support of an AI agent, planned thoroughly with a tight feedback loop and reviewed at each step. While we've tested extensively, edge cases may exist. Use in production at your own discretion, and please [report any issues](https://github.com/msmps/pilotty/issues) you find!

pilotty enables AI agents to interact with terminal applications through a simple command-line interface. It manages pseudo-terminal (PTY) sessions with full VT100 terminal emulation, captures screen state, and provides keyboard/mouse input for navigating terminal user interfaces. Think of it as headless terminal automation for AI workflows.

## Features

- **PTY (Pseudo-Terminal) Management**: Spawn and manage terminal applications in background sessions
- **Terminal Emulation**: Full VT100 emulation for accurate screen capture and state tracking
- **Keyboard Navigation**: Interact with TUIs using Tab, Enter, arrow keys, and key combos
- **AI-Friendly Output**: Clean JSON responses with actionable suggestions on errors
- **Multi-Session**: Run multiple terminal apps simultaneously in isolated sessions
- **Zero Config**: Daemon auto-starts on first command, auto-stops after 5 minutes idle

## Why pilotty?

[agent-browser](https://github.com/vercel-labs/agent-browser) by Vercel Labs lets AI agents control web browsers. pilotty does the same for terminals.

**Origin story:** Built to solve a personal problem, pilotty was created to enable AI agents to interact with [OpenTUI](https://github.com/anomalyco/opentui) interfaces and control [OpenCode](https://github.com/anomalyco/opencode) programmatically. If you're building TUIs or working with terminal applications, pilotty lets AI navigate them just like a human would.

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

# Click at specific coordinates (row, col)
pilotty click 10 5

# List active sessions
pilotty list-sessions

# Stop the daemon
pilotty stop
```

## Commands

### Session Management

```bash
pilotty spawn <command>           # Spawn a TUI app (e.g., pilotty spawn vim file.txt)
pilotty spawn --name myapp <cmd>  # Spawn with a custom session name
pilotty spawn --cwd /path cmd     # Spawn in a specific working directory
pilotty kill                      # Kill default session
pilotty kill -s myapp             # Kill specific session
pilotty list-sessions             # List all active sessions
pilotty stop                      # Stop the daemon and all sessions
pilotty daemon                    # Manually start daemon (usually auto-starts)
pilotty examples                  # Show end-to-end workflow example
```

### Screen Capture

```bash
pilotty snapshot                  # Full JSON with text
pilotty snapshot --format compact # JSON without text field
pilotty snapshot --format text    # Plain text with cursor indicator

# Wait for screen to change before returning (no more manual sleep!)
pilotty snapshot --await-change $HASH           # Block until hash differs
pilotty snapshot --await-change $HASH --settle 100  # Then wait for stability
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

# Key sequences (space-separated keys sent in order)
pilotty key "Ctrl+X m"            # Emacs chord: Ctrl+X then m
pilotty key "Escape : w q Enter"  # vim :wq sequence
pilotty key "a b c" --delay 50    # Send a, b, c with 50ms delay between
```

### Interaction

```bash
pilotty click 10 5                # Click at row 10, col 5
pilotty scroll up                 # Scroll up 1 line
pilotty scroll down 5             # Scroll down 5 lines
```

### Terminal Control

```bash
pilotty resize 120 40             # Resize terminal to 120x40
pilotty wait-for "Ready"          # Wait for text to appear
pilotty wait-for "Error" --regex  # Wait for regex pattern
pilotty wait-for "Done" -t 5000   # Wait with 5s timeout

# Wait for screen changes (preferred over sleep)
HASH=$(pilotty snapshot | jq '.content_hash')
pilotty key Enter
pilotty snapshot --await-change $HASH --settle 50  # Wait for change + 50ms stability
```

## Snapshot Output

The `snapshot` command returns structured data about the terminal screen:

```json
{
  "snapshot_id": 42,
  "size": { "cols": 80, "rows": 24 },
  "cursor": { "row": 5, "col": 10, "visible": true },
  "text": "Options: [x] Enable  [ ] Debug\nActions: [OK] [Cancel]",
  "elements": [
    { "kind": "toggle", "row": 0, "col": 9, "width": 3, "text": "[x]", "confidence": 1.0, "checked": true },
    { "kind": "toggle", "row": 0, "col": 22, "width": 3, "text": "[ ]", "confidence": 1.0, "checked": false },
    { "kind": "button", "row": 1, "col": 9, "width": 4, "text": "[OK]", "confidence": 0.8 },
    { "kind": "button", "row": 1, "col": 14, "width": 8, "text": "[Cancel]", "confidence": 0.8 }
  ],
  "content_hash": 12345678901234567890
}
```

## UI Elements (Contextual)

pilotty automatically detects interactive UI elements in terminal applications. Elements provide **read-only context** to help understand UI structure, with position data (row, col) for use with the click command.

**Use keyboard navigation (`pilotty key Tab`, `pilotty key Enter`, `pilotty type "text"`) for reliable TUI interaction** rather than element-based actions, as UI element detection depends on visual patterns that may disappear after interaction.

### Element Kinds

| Kind | Detection Patterns | Confidence |
|------|-------------------|------------|
| **button** | Inverse video, `[OK]`, `<Cancel>` | 1.0 / 0.8 |
| **input** | Cursor position, `____` underscores | 1.0 / 0.6 |
| **toggle** | `[x]`, `[ ]`, `☑`, `☐` | 1.0 |

### Element Fields

| Field | Description |
|-------|-------------|
| `kind` | Element type: `button`, `input`, or `toggle` |
| `row` | Row position (0-based) |
| `col` | Column position (0-based) |
| `width` | Width in terminal cells |
| `text` | Text content of the element |
| `confidence` | Detection confidence (0.0-1.0) |
| `focused` | Whether element has focus (only present if true) |
| `checked` | Toggle state (only present for toggles) |

### Wait for Screen Changes

The `--await-change` flag solves the fundamental problem of TUI automation: **"How long should I wait after an action?"**

Instead of guessing sleep durations (too short = race condition, too long = slow), wait for the screen to actually change:

```bash
# Capture baseline hash
HASH=$(pilotty snapshot | jq '.content_hash')

# Perform action
pilotty key Enter

# Wait for screen to change (blocks until hash differs)
pilotty snapshot --await-change $HASH

# Or wait for screen to stabilize (useful for apps that render progressively)
pilotty snapshot --await-change $HASH --settle 100  # Wait 100ms after last change
```

**Flags:**
- `--await-change <HASH>`: Block until `content_hash` differs from this value
- `--settle <MS>`: After change detected, wait for screen to be stable for this many ms
- `--timeout <MS>`: Maximum wait time (default: 30000)

**Why this matters:**
- No more flaky automation due to race conditions
- No more slow scripts due to conservative sleep values  
- Works regardless of how fast/slow the target app is
- The `--settle` flag handles apps that render progressively

### Manual Change Detection

For manual polling, use `content_hash` directly:

```bash
# Get initial snapshot
SNAP1=$(pilotty snapshot)
HASH1=$(echo "$SNAP1" | jq -r '.content_hash')

# Perform some action
pilotty key Tab

# Check if screen changed
SNAP2=$(pilotty snapshot)
HASH2=$(echo "$SNAP2" | jq -r '.content_hash')

if [ "$HASH1" != "$HASH2" ]; then
  echo "Screen content changed"
fi
```

### Workflow Example

```bash
# 1. Spawn a TUI with dialog elements
pilotty spawn dialog --yesno "Continue?" 10 40

# 2. Wait for dialog to render
pilotty wait-for "Continue"

# 3. Get snapshot with elements (for context)
pilotty snapshot | jq '.elements'
# Shows detected buttons, helps understand UI structure

# 4. Navigate and interact with keyboard (reliable approach)
pilotty key Tab      # Move to next element
pilotty key Enter    # Activate selected element
```

## Sessions

Each session is an isolated terminal with its own:
- PTY (pseudo-terminal)
- Screen buffer
- Child process

```bash
# Run multiple apps (--name must come before the command)
pilotty spawn --name monitoring htop
pilotty spawn --name editor vim file.txt

# Run app in a specific directory (useful for project-specific configs)
pilotty spawn --cwd /path/to/project --name myapp bun src/index.tsx

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
pilotty spawn --name monitoring htop
pilotty spawn --name editor vim
```

> **Important:** The `--name` flag must come **before** the command. Everything after the command is passed as arguments to that command.

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

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PILOTTY_SESSION` | Default session name |
| `PILOTTY_SOCKET_DIR` | Override socket directory |
| `RUST_LOG` | Logging level (e.g., `debug`, `info`) |

## Usage with AI Agents

### AI Coding Assistants

Add the skill to your AI coding assistant for richer context:

```bash
npx skills add msmps/pilotty
```

This works with Claude Code, Codex, Cursor, Gemini CLI, GitHub Copilot, Goose, OpenCode, and Windsurf.

### Just Ask the Agent

The simplest approach - just tell your agent to use it:

```
Use pilotty to interact with vim. Run pilotty --help to see available commands.
```

The `--help` output is comprehensive and most agents can figure it out from there.

### AGENTS.md / CLAUDE.md

For more consistent results, add to your project or global instructions file:

```markdown
## Terminal Automation

Use `pilotty` for TUI automation. Run `pilotty --help` for all commands.

Core workflow:
1. `pilotty spawn <command>` - Start a TUI application
2. `pilotty snapshot` - Get screen state with cursor position
3. `pilotty key Tab` / `pilotty type "text"` - Navigate and interact
4. Re-snapshot after screen changes
```

### Example Workflow

```bash
# 1. Spawn the application
pilotty spawn vim myfile.txt

# 2. Wait for it to be ready
pilotty wait-for "myfile.txt"

# 3. Take a snapshot to understand the screen and capture hash
HASH=$(pilotty snapshot | jq '.content_hash')

# 4. Navigate using keyboard commands
pilotty key i                    # Enter insert mode
pilotty type "Hello, World!"
pilotty key Escape

# 5. Wait for screen to update, then save (no manual sleep needed!)
pilotty snapshot --await-change $HASH --settle 50
pilotty key "Escape : w q Enter"  # vim :wq sequence

# 6. Verify vim exited
pilotty list-sessions
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
| **Sequences** | `"Ctrl+X m"`, `"Escape : w q Enter"` | Space-separated keys |

### Key Sequences

Send multiple keys in order with optional delay between them:

```bash
# Emacs-style chords
pilotty key "Ctrl+X Ctrl+S"       # Save in Emacs
pilotty key "Ctrl+X m"            # Compose mail in Emacs

# vim command sequences
pilotty key "Escape : w q Enter"  # Save and quit vim
pilotty key "g g d G"             # Delete entire file in vim

# With inter-key delay (useful for slow TUIs)
pilotty key "Tab Tab Enter" --delay 100   # Navigate with 100ms between keys
```

The `--delay` flag specifies milliseconds between keys (max 10000ms, default 0).

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
