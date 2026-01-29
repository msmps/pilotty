---
name: pilotty
description: Automates terminal TUI applications (vim, htop, lazygit, dialog) through managed PTY sessions. Use when the user needs to interact with terminal apps, edit files in vim/nano, navigate TUI menus, click terminal buttons/checkboxes, or automate CLI workflows with interactive prompts.
allowed-tools: Bash(pilotty:*)
---

# Terminal Automation with pilotty

## CRITICAL: Argument Positioning

**All flags (`--name`, `-s`, `--format`, etc.) MUST come BEFORE positional arguments:**

```bash
# CORRECT - flags before command/arguments
pilotty spawn --name myapp vim file.txt
pilotty key -s myapp Enter
pilotty snapshot -s myapp --format text

# WRONG - flags after command (they get passed to the app, not pilotty!)
pilotty spawn vim file.txt --name myapp   # FAILS: --name goes to vim
pilotty key Enter -s myapp                # FAILS: -s goes nowhere useful
```

This is the #1 cause of agent failures. When in doubt: **flags first, then command/args**.

---

## Quick start

```bash
pilotty spawn vim file.txt        # Start TUI app in managed session
pilotty wait-for "file.txt"       # Wait for app to be ready
pilotty snapshot                  # Get screen state with UI elements
pilotty key i                     # Enter insert mode
pilotty type "Hello, World!"      # Type text
pilotty key Escape                # Exit insert mode
pilotty kill                      # End session
```

## Core workflow

1. **Spawn**: `pilotty spawn <command>` starts the app in a background PTY
2. **Wait**: `pilotty wait-for <text>` ensures the app is ready
3. **Snapshot**: `pilotty snapshot` returns screen state with detected UI elements
4. **Understand**: Parse `elements[]` to identify buttons, inputs, toggles
5. **Interact**: Use keyboard commands (`key`, `type`) to navigate and interact
6. **Re-snapshot**: Check `content_hash` to detect screen changes

## Commands

### Session management

```bash
pilotty spawn <command>           # Start TUI app (e.g., pilotty spawn htop)
pilotty spawn --name myapp <cmd>  # Start with custom session name (--name before command)
pilotty kill                      # Kill default session
pilotty kill -s myapp             # Kill specific session
pilotty list-sessions             # List all active sessions
pilotty daemon                    # Manually start daemon (usually auto-starts)
pilotty shutdown                  # Stop daemon and all sessions
pilotty examples                  # Show end-to-end workflow example
```

### Screen capture

```bash
pilotty snapshot                  # Full JSON with text content and elements
pilotty snapshot --format compact # JSON without text field
pilotty snapshot --format text    # Plain text with cursor indicator
pilotty snapshot -s myapp         # Snapshot specific session

# Wait for screen to change (eliminates need for sleep!)
HASH=$(pilotty snapshot | jq '.content_hash')
pilotty key Enter
pilotty snapshot --await-change $HASH           # Block until screen changes
pilotty snapshot --await-change $HASH --settle 50  # Wait for 50ms stability
```

### Input

```bash
pilotty type "hello"              # Type text at cursor
pilotty type -s myapp "text"      # Type in specific session

pilotty key Enter                 # Press Enter
pilotty key Ctrl+C                # Send interrupt
pilotty key Escape                # Send Escape
pilotty key Tab                   # Send Tab
pilotty key F1                    # Function key
pilotty key Alt+F                 # Alt combination
pilotty key Up                    # Arrow key
pilotty key -s myapp Ctrl+S       # Key in specific session

# Key sequences (space-separated, sent in order)
pilotty key "Ctrl+X m"            # Emacs chord: Ctrl+X then m
pilotty key "Escape : w q Enter"  # vim :wq sequence
pilotty key "a b c" --delay 50    # Send a, b, c with 50ms delay
pilotty key -s myapp "Tab Tab Enter"  # Sequence in specific session
```

### Interaction

```bash
pilotty click 5 10                # Click at row 5, col 10
pilotty click -s myapp 10 20      # Click in specific session
pilotty scroll up                 # Scroll up 1 line
pilotty scroll down 5             # Scroll down 5 lines
pilotty scroll up 10 -s myapp     # Scroll in specific session
```

### Terminal control

```bash
pilotty resize 120 40             # Resize terminal to 120 cols x 40 rows
pilotty resize 80 24 -s myapp     # Resize specific session

pilotty wait-for "Ready"          # Wait for text to appear (30s default)
pilotty wait-for "Error" -r       # Wait for regex pattern
pilotty wait-for "Done" -t 5000   # Wait with 5s timeout
pilotty wait-for "~" -s editor    # Wait in specific session
```

## Global options

| Option | Description |
|--------|-------------|
| `-s, --session <name>` | Target specific session (default: "default") |
| `--format <fmt>` | Snapshot format: full, compact, text |
| `-t, --timeout <ms>` | Timeout for wait-for and await-change (default: 30000) |
| `-r, --regex` | Treat wait-for pattern as regex |
| `--name <name>` | Session name for spawn command |
| `--delay <ms>` | Delay between keys in a sequence (default: 0, max: 10000) |
| `--await-change <hash>` | Block snapshot until content_hash differs |
| `--settle <ms>` | Wait for screen to be stable for this many ms (default: 0) |

### Environment variables

```bash
PILOTTY_SESSION="mysession"       # Default session name
PILOTTY_SOCKET_DIR="/tmp/pilotty" # Override socket directory
RUST_LOG="debug"                  # Enable debug logging
```

## Snapshot Output

The `snapshot` command returns structured JSON with detected UI elements:

```json
{
  "snapshot_id": 42,
  "size": { "cols": 80, "rows": 24 },
  "cursor": { "row": 5, "col": 10, "visible": true },
  "text": "Settings:\n  [x] Notifications  [ ] Dark mode\n  [Save]  [Cancel]",
  "elements": [
    { "kind": "toggle", "row": 1, "col": 2, "width": 3, "text": "[x]", "confidence": 1.0, "checked": true },
    { "kind": "toggle", "row": 1, "col": 20, "width": 3, "text": "[ ]", "confidence": 1.0, "checked": false },
    { "kind": "button", "row": 2, "col": 2, "width": 6, "text": "[Save]", "confidence": 0.8 },
    { "kind": "button", "row": 2, "col": 10, "width": 8, "text": "[Cancel]", "confidence": 0.8 }
  ],
  "content_hash": 12345678901234567890
}
```

Use `--format text` for a plain text view with cursor indicator:

```
--- Terminal 80x24 | Cursor: (5, 10) ---
bash-3.2$ [_]
```

The `[_]` shows cursor position. Use the text content to understand screen state and navigate with keyboard commands.

---

## Element Detection

pilotty automatically detects interactive UI elements in terminal applications. Elements provide **read-only context** to help understand UI structure.

### Element Kinds

| Kind | Detection Patterns | Confidence | Fields |
|------|-------------------|------------|--------|
| **toggle** | `[x]`, `[ ]`, `[*]`, `☑`, `☐` | 1.0 | `checked: bool` |
| **button** | Inverse video, `[OK]`, `<Cancel>`, `(Submit)` | 1.0 / 0.8 | `focused: bool` (if true) |
| **input** | Cursor position, `____` underscores | 1.0 / 0.6 | `focused: bool` (if true) |

### Element Fields

| Field | Type | Description |
|-------|------|-------------|
| `kind` | string | Element type: `button`, `input`, or `toggle` |
| `row` | number | Row position (0-based from top) |
| `col` | number | Column position (0-based from left) |
| `width` | number | Width in terminal cells (CJK chars = 2) |
| `text` | string | Text content of the element |
| `confidence` | number | Detection confidence (0.0-1.0) |
| `focused` | bool | Whether element has focus (only present if true) |
| `checked` | bool | Toggle state (only present for toggles) |

### Confidence Levels

| Confidence | Meaning |
|------------|---------|
| **1.0** | High confidence: Cursor position, inverse video, checkbox patterns |
| **0.8** | Medium confidence: Bracket patterns `[OK]`, `<Cancel>` |
| **0.6** | Lower confidence: Underscore input fields `____` |

### Wait for Screen Changes (Recommended)

**Stop guessing sleep durations!** Use `--await-change` to wait for the screen to actually update:

```bash
# Capture baseline hash
HASH=$(pilotty snapshot | jq '.content_hash')

# Perform action
pilotty key Enter

# Wait for screen to change (blocks until hash differs)
pilotty snapshot --await-change $HASH

# Or wait for screen to stabilize (for apps that render progressively)
pilotty snapshot --await-change $HASH --settle 100
```

**Flags:**
| Flag | Description |
|------|-------------|
| `--await-change <HASH>` | Block until `content_hash` differs from this value |
| `--settle <MS>` | After change detected, wait for screen to be stable for MS |
| `-t, --timeout <MS>` | Maximum wait time (default: 30000) |

**Why this is better than sleep:**
- `sleep 1` is a guess - too short causes race conditions, too long slows automation
- `--await-change` waits exactly as long as needed - no more, no less
- `--settle` handles apps that render progressively (show partial, then complete)

### Waiting for Streaming AI Responses

When interacting with AI-powered TUIs (like opencode, etc.) that stream responses, you need a longer `--settle` time since the screen keeps updating as tokens arrive:

```bash
# 1. Capture hash before sending prompt
HASH=$(pilotty snapshot -s myapp | jq -r '.content_hash')

# 2. Type prompt and submit
pilotty type -s myapp "write me a poem about ai agents"
pilotty key -s myapp Enter

# 3. Wait for streaming response to complete
#    - Use longer settle (2-3s) since AI apps pause between chunks
#    - Extend timeout for long responses (60s+)
pilotty snapshot -s myapp --await-change "$HASH" --settle 3000 -t 60000

# 4. Response may be scrolled - scroll up if needed to see full output
pilotty scroll -s myapp up 10
pilotty snapshot -s myapp --format text
```

**Key parameters for streaming:**
- `--settle 2000-3000`: AI responses have pauses between chunks; 2-3 seconds ensures streaming is truly done
- `-t 60000`: Extend timeout beyond the 30s default for longer generations
- The settle timer resets on each screen change, so it naturally waits until streaming stops

### Manual Change Detection

For manual polling (not recommended), use `content_hash` directly:

```bash
# Get initial state
SNAP1=$(pilotty snapshot)
HASH1=$(echo "$SNAP1" | jq -r '.content_hash')

# Perform action
pilotty key Tab

# Check if screen changed
SNAP2=$(pilotty snapshot)
HASH2=$(echo "$SNAP2" | jq -r '.content_hash')

if [ "$HASH1" != "$HASH2" ]; then
  echo "Screen changed - re-analyze elements"
fi
```

### Using Elements Effectively

Elements are **read-only context** for understanding the UI. Use **keyboard navigation** for reliable interaction:

```bash
# 1. Get snapshot to understand UI structure
pilotty snapshot | jq '.elements'
# Output shows toggles (checked/unchecked) and buttons with positions

# 2. Navigate and interact with keyboard (reliable approach)
pilotty key Tab          # Move to next element
pilotty key Space        # Toggle checkbox
pilotty key Enter        # Activate button

# 3. Verify state changed
pilotty snapshot | jq '.elements[] | select(.kind == "toggle")'
```

**Key insight**: Use elements to understand WHAT is on screen, use keyboard to interact with it.

---

## Navigation Approach

pilotty uses keyboard-first navigation, just like a human would:

```bash
# 1. Take snapshot to see the screen
pilotty snapshot --format text

# 2. Navigate using keyboard
pilotty key Tab           # Move to next element
pilotty key Enter         # Activate/select
pilotty key Escape        # Cancel/back
pilotty key Up            # Move up in list/menu
pilotty key Space         # Toggle checkbox

# 3. Type text when needed
pilotty type "search term"
pilotty key Enter

# 4. Click at coordinates for mouse-enabled TUIs
pilotty click 5 10        # Click at row 5, col 10
```

**Key insight**: Parse the snapshot text and elements to understand what's on screen, then use keyboard commands to navigate. This works reliably across all TUI applications.

---

## Example: Edit file with vim

```bash
# 1. Spawn vim
pilotty spawn --name editor vim /tmp/hello.txt

# 2. Wait for vim to load and capture baseline hash
pilotty wait-for -s editor "hello.txt"
HASH=$(pilotty snapshot -s editor | jq '.content_hash')

# 3. Enter insert mode
pilotty key -s editor i

# 4. Type content
pilotty type -s editor "Hello from pilotty!"

# 5. Wait for screen to update, then exit (no sleep needed!)
pilotty snapshot -s editor --await-change $HASH --settle 50
pilotty key -s editor "Escape : w q Enter"

# 6. Verify session ended
pilotty list-sessions
```

Alternative using individual keys:
```bash
pilotty key -s editor Escape
pilotty type -s editor ":wq"
pilotty key -s editor Enter
```

## Example: Dialog checklist interaction

```bash
# 1. Spawn dialog checklist (--name before command)
pilotty spawn --name opts dialog --checklist "Select features:" 12 50 4 \
    "notifications" "Push notifications" on \
    "darkmode" "Dark mode theme" off \
    "autosave" "Auto-save documents" on \
    "telemetry" "Usage analytics" off

# 2. Wait for dialog to render (use await-change, not sleep!)
pilotty snapshot -s opts --settle 200  # Wait for initial render to stabilize

# 3. Get snapshot and examine elements, capture hash
SNAP=$(pilotty snapshot -s opts)
echo "$SNAP" | jq '.elements[] | select(.kind == "toggle")'
HASH=$(echo "$SNAP" | jq '.content_hash')

# 4. Navigate to "darkmode" and toggle it
pilotty key -s opts Down      # Move to second option
pilotty key -s opts Space     # Toggle it on

# 5. Wait for change and verify
pilotty snapshot -s opts --await-change $HASH | jq '.elements[] | select(.kind == "toggle") | {text, checked}'

# 6. Confirm selection
pilotty key -s opts Enter

# 7. Clean up
pilotty kill -s opts
```

## Example: Form filling with elements

```bash
# 1. Spawn a form application
pilotty spawn --name form my-form-app

# 2. Get snapshot to understand form structure
pilotty snapshot -s form | jq '.elements'
# Shows inputs, toggles, and buttons with positions for click command

# 3. Tab to first input (likely already focused)
pilotty type -s form "myusername"

# 4. Tab to password field
pilotty key -s form Tab
pilotty type -s form "mypassword"

# 5. Tab to remember me and toggle
pilotty key -s form Tab
pilotty key -s form Space

# 6. Tab to Login and activate
pilotty key -s form Tab
pilotty key -s form Enter

# 7. Check result
pilotty snapshot -s form --format text
```

## Example: Monitor with htop

```bash
# 1. Spawn htop
pilotty spawn --name monitor htop

# 2. Wait for display
pilotty wait-for -s monitor "CPU"

# 3. Take snapshot to see current state
pilotty snapshot -s monitor --format text

# 4. Send commands
pilotty key -s monitor F9    # Kill menu
pilotty key -s monitor q     # Quit

# 5. Kill session
pilotty kill -s monitor
```

## Example: Interact with AI TUI (opencode, etc.)

AI-powered TUIs stream responses, requiring special handling:

```bash
# 1. Spawn the AI app
pilotty spawn --name ai opencode

# 2. Wait for the prompt to be ready
pilotty wait-for -s ai "Ask anything" -t 15000

# 3. Capture baseline hash
HASH=$(pilotty snapshot -s ai | jq -r '.content_hash')

# 4. Type prompt and submit
pilotty type -s ai "explain the architecture of this codebase"
pilotty key -s ai Enter

# 5. Wait for streaming response to complete
#    - settle=3000: Wait 3s of no changes to ensure streaming is done
#    - timeout=60000: Allow up to 60s for long responses
pilotty snapshot -s ai --await-change "$HASH" --settle 3000 -t 60000 --format text

# 6. If response is long and scrolled, scroll up to see full output
pilotty scroll -s ai up 20
pilotty snapshot -s ai --format text

# 7. Clean up
pilotty kill -s ai
```

**Gotchas with AI apps:**
- Use `--settle 2000-3000` because AI responses pause between chunks
- Extend timeout with `-t 60000` for complex prompts
- Long responses may scroll the terminal; use `scroll up` to see the beginning
- The settle timer resets on each screen update, so it waits for true completion

---

## Sessions

Each session is isolated with its own:
- PTY (pseudo-terminal)
- Screen buffer
- Child process

```bash
# Run multiple apps (--name must come before the command)
pilotty spawn --name monitoring htop
pilotty spawn --name editor vim file.txt

# Target specific session
pilotty snapshot -s monitoring
pilotty key -s editor Ctrl+S

# List all
pilotty list-sessions

# Kill specific
pilotty kill -s editor
```

The first session spawned without `--name` is automatically named `default`.

> **Important:** The `--name` flag must come **before** the command. Everything after the command is passed as arguments to that command.

## Daemon Architecture

pilotty uses a background daemon for session management:

- **Auto-start**: Daemon starts on first command
- **Auto-stop**: Shuts down after 5 minutes with no sessions
- **Session cleanup**: Sessions removed when process exits (within 500ms)
- **Shared state**: Multiple CLI calls share sessions

You rarely need to manage the daemon manually.

## Error Handling

Errors include actionable suggestions:

```json
{
  "code": "SESSION_NOT_FOUND",
  "message": "Session 'abc123' not found",
  "suggestion": "Run 'pilotty list-sessions' to see available sessions"
}
```

```json
{
  "code": "SPAWN_FAILED",
  "message": "Failed to spawn process: command not found",
  "suggestion": "Check that the command exists and is in PATH"
}
```

---

## Common Patterns

### Reliable action + wait (recommended)

```bash
# The pattern: capture hash, act, await change
HASH=$(pilotty snapshot | jq '.content_hash')
pilotty key Enter
pilotty snapshot --await-change $HASH --settle 50

# This replaces fragile patterns like:
# pilotty key Enter && sleep 1 && pilotty snapshot  # BAD: guessing
```

### Wait then act

```bash
pilotty spawn my-app
pilotty wait-for "Ready"    # Ensure app is ready
pilotty snapshot            # Then snapshot
```

### Check state before action

```bash
pilotty snapshot --format text | grep "Error"  # Check for errors
pilotty key Enter                               # Then proceed
```

### Check for specific element

```bash
# Check if the first toggle is checked
pilotty snapshot | jq '.elements[] | select(.kind == "toggle") | {text, checked}' | head -1

# Find element at specific position
pilotty snapshot | jq '.elements[] | select(.row == 5 and .col == 10)'
```

### Retry on timeout

```bash
pilotty wait-for "Ready" -t 5000 || {
  pilotty snapshot --format text   # Check what's on screen
  # Adjust approach based on actual state
}
```

---

## Deep-dive Documentation

For detailed patterns and edge cases, see:

| Reference | Description |
|-----------|-------------|
| [references/session-management.md](references/session-management.md) | Multi-session patterns, isolation, cleanup |
| [references/key-input.md](references/key-input.md) | Complete key combinations reference |
| [references/element-detection.md](references/element-detection.md) | Detection rules, confidence, patterns |

## Ready-to-use Templates

Executable workflow scripts:

| Template | Description |
|----------|-------------|
| [templates/vim-workflow.sh](templates/vim-workflow.sh) | Edit file with vim, save, exit |
| [templates/dialog-interaction.sh](templates/dialog-interaction.sh) | Handle dialog/whiptail prompts |
| [templates/multi-session.sh](templates/multi-session.sh) | Parallel TUI orchestration |
| [templates/element-detection.sh](templates/element-detection.sh) | Element detection demo |

Usage:
```bash
./templates/vim-workflow.sh /tmp/myfile.txt "File content here"
./templates/dialog-interaction.sh
./templates/multi-session.sh
./templates/element-detection.sh
```
