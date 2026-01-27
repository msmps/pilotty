---
name: pilotty
description: Automates terminal TUI applications (vim, htop, lazygit, dialog) through managed PTY sessions. Use when the user needs to interact with terminal apps, edit files in vim/nano, navigate TUI menus, click terminal buttons/checkboxes, or automate CLI workflows with interactive prompts.
allowed-tools: Bash(pilotty:*)
---

# Terminal Automation with pilotty

## Quick start

```bash
pilotty spawn vim file.txt        # Start TUI app in managed session
pilotty wait-for "file.txt"       # Wait for app to be ready
pilotty snapshot                  # Get screen state with refs
pilotty click @e1                 # Click element by ref
pilotty kill                      # End session
```

## Core workflow

1. **Spawn**: `pilotty spawn <command>` starts the app in a background PTY
2. **Wait**: `pilotty wait-for <text>` ensures the app is ready
3. **Snapshot**: `pilotty snapshot` returns screen state with interactive regions as refs (`@e1`, `@e2`)
4. **Interact**: Use refs to click elements, or use `type`/`key` for input
5. **Re-snapshot**: After significant screen changes, snapshot again to get updated refs

**Critical**: Refs are tied to screen state. Re-snapshot after navigation or content changes.

## Commands

### Session management

```bash
pilotty spawn <command>           # Start TUI app (e.g., pilotty spawn htop)
pilotty spawn <cmd> --name myapp  # Start with custom session name
pilotty kill                      # Kill default session
pilotty kill -s myapp             # Kill specific session
pilotty list-sessions             # List all active sessions
pilotty daemon                    # Manually start daemon (usually auto-starts)
pilotty stop                      # Stop daemon and all sessions
pilotty examples                  # Show end-to-end workflow example
```

### Screen capture

```bash
pilotty snapshot                  # Full JSON with regions and text
pilotty snapshot --format compact # Compact format with inline refs
pilotty snapshot --format text    # Plain text only, no metadata
pilotty snapshot -s myapp         # Snapshot specific session
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
```

### Interaction

```bash
pilotty click @e1                 # Click region by ref
pilotty click -s myapp @e3        # Click in specific session
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
| `-t, --timeout <ms>` | Timeout for wait-for (default: 30000) |
| `-r, --regex` | Treat wait-for pattern as regex |
| `--name <name>` | Session name for spawn command |

### Environment variables

```bash
PILOTTY_SESSION="mysession"       # Default session name
PILOTTY_SOCKET_DIR="/tmp/pilotty" # Override socket directory
RUST_LOG="debug"                  # Enable debug logging
```

## Snapshot output

The `snapshot` command returns structured JSON:

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

### Region types

pilotty automatically detects these interactive elements:

| Type | Pattern | Example |
|------|---------|---------|
| `button` | Bracketed text | `[ OK ]`, `< Save >` |
| `checkbox` | Square brackets with x/space | `[x] Enable`, `[ ] Disable` |
| `radio_button` | Parens with */space | `(*) Option`, `( ) Other` |
| `menu_item` | Shortcut or inverse video | `(F)ile`, highlighted text |
| `link` | Underlined text | URLs, clickable references |
| `text_input` | Box-drawn input fields | Dialog text inputs |
| `scrollable_area` | Scroll regions | Detected contextually |
| `unknown` | Unclassified boxes | Bordered regions |

## Using refs

Refs (`@e1`, `@e2`, etc.) are stable identifiers for interactive elements.

```bash
# 1. Get snapshot with refs
pilotty snapshot
# Output includes: "@e1" [button] "[ OK ]", "@e2" [button] "[ Cancel ]"

# 2. Click by ref
pilotty click @e1

# 3. Re-snapshot if screen changed
pilotty snapshot
```

**Ref stability**: Refs persist when content/position are similar. After major screen changes (navigation, new dialog), re-snapshot to get fresh refs.

## Example: Edit file with vim

```bash
# 1. Spawn vim
pilotty spawn --name editor vim /tmp/hello.txt

# 2. Wait for vim to load
pilotty wait-for -s editor "hello.txt"

# 3. Enter insert mode
pilotty key -s editor i

# 4. Type content
pilotty type -s editor "Hello from pilotty!"

# 5. Exit insert mode
pilotty key -s editor Escape

# 6. Save and quit
pilotty type -s editor ":wq"
pilotty key -s editor Enter

# 7. Verify session ended
pilotty list-sessions
```

## Example: Dialog interaction

```bash
# 1. Spawn dialog
pilotty spawn dialog --yesno "Continue?" 10 40

# 2. Get snapshot to see buttons
pilotty snapshot
# Output: @e1 [button] "< Yes >", @e2 [button] "< No >"

# 3. Click Yes
pilotty click @e1

# Or navigate with keys
pilotty key Tab      # Move to next button
pilotty key Enter    # Activate
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

## Sessions

Each session is isolated with its own:
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

# List all
pilotty list-sessions

# Kill specific
pilotty kill -s editor
```

The first session spawned without `--name` is automatically named `default`.

## Daemon architecture

pilotty uses a background daemon for session management:

- **Auto-start**: Daemon starts on first command
- **Auto-stop**: Shuts down after 5 minutes with no sessions
- **Session cleanup**: Sessions removed when process exits (within 500ms)
- **Shared state**: Multiple CLI calls share sessions

You rarely need to manage the daemon manually.

## Error handling

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
  "code": "REF_NOT_FOUND",
  "message": "Region '@e5' not found",
  "suggestion": "Run 'pilotty snapshot' to get updated refs. Available: @e1, @e2"
}
```

## Common patterns

### Wait then act

```bash
pilotty spawn my-app
pilotty wait-for "Ready"    # Ensure app is ready
pilotty snapshot            # Then snapshot
```

### Check state before action

```bash
pilotty snapshot --format text | grep "Error"  # Check for errors
pilotty click @e1                               # Then proceed
```

### Retry on stale ref

```bash
pilotty click @e1 || {
  pilotty snapshot          # Ref stale, re-snapshot
  pilotty click @e1         # Retry with fresh ref
}
```

## Deep-dive documentation

For detailed patterns and edge cases, see:

| Reference | Description |
|-----------|-------------|
| [references/region-refs.md](references/region-refs.md) | Ref lifecycle, invalidation, troubleshooting |
| [references/session-management.md](references/session-management.md) | Multi-session patterns, isolation, cleanup |
| [references/key-input.md](references/key-input.md) | Complete key combinations reference |

## Ready-to-use templates

Executable workflow scripts:

| Template | Description |
|----------|-------------|
| [templates/vim-workflow.sh](templates/vim-workflow.sh) | Edit file with vim, save, exit |
| [templates/dialog-interaction.sh](templates/dialog-interaction.sh) | Handle dialog/whiptail prompts |
| [templates/multi-session.sh](templates/multi-session.sh) | Parallel TUI orchestration |

Usage:
```bash
./templates/vim-workflow.sh /tmp/myfile.txt "File content here"
./templates/dialog-interaction.sh
./templates/multi-session.sh
```
