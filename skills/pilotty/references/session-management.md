# Session Management

pilotty manages multiple isolated terminal sessions, each running its own TUI application with independent state.

## Session Basics

Each session has:
- **PTY**: Pseudo-terminal for the application
- **Screen buffer**: Terminal emulator state
- **Region tracker**: Detected interactive elements and refs
- **Child process**: The running application

## Creating Sessions

### Default Session

The first spawn without `--name` creates the `default` session:

```bash
pilotty spawn htop
# Creates session named "default"

pilotty snapshot
# Snapshots the "default" session
```

### Named Sessions

Use `--name` for multiple concurrent sessions:

```bash
pilotty spawn htop --name monitoring
pilotty spawn vim file.txt --name editor
pilotty spawn lazygit --name git
```

### Session Naming Rules

- Names are sanitized (alphanumeric, hyphens, underscores)
- Path traversal attempts (`../`) are rejected
- Names must be unique per daemon instance

## Targeting Sessions

Use `-s` or `--session` to target a specific session:

```bash
# Snapshot specific session
pilotty snapshot -s monitoring

# Send key to specific session
pilotty key -s editor Ctrl+S

# Click in specific session
pilotty click -s git @e1

# Kill specific session
pilotty kill -s monitoring
```

Without `-s`, commands target the most recently used session (or `default`).

## Listing Sessions

```bash
pilotty list-sessions
```

Output:
```json
{
  "sessions": [
    { "id": "abc123", "name": "monitoring", "command": "htop" },
    { "id": "def456", "name": "editor", "command": "vim file.txt" },
    { "id": "ghi789", "name": "git", "command": "lazygit" }
  ]
}
```

## Session Lifecycle

### Spawn

```bash
pilotty spawn --name myapp my-command arg1 arg2
```

1. Daemon creates PTY
2. Forks child process with command
3. Initializes terminal emulator (default: 80x24)
4. Returns session ID

### Active Use

While a session is active:
- Screen buffer updates on process output
- Regions are re-detected on significant changes
- Refs remain stable until screen changes

### Process Exit

When the child process exits:
- Session is marked for cleanup
- Cleanup happens within 500ms
- Refs become invalid
- Session is removed from list

### Manual Kill

```bash
pilotty kill -s myapp
```

Sends SIGTERM to the child process, then cleans up.

## Multi-Session Patterns

### Parallel Monitoring

Run multiple apps and switch between them:

```bash
# Start apps
pilotty spawn htop --name cpu
pilotty spawn iotop --name io
pilotty spawn nethogs --name net

# Check each
pilotty snapshot -s cpu --format text
pilotty snapshot -s io --format text
pilotty snapshot -s net --format text

# Clean up
pilotty kill -s cpu
pilotty kill -s io
pilotty kill -s net
```

### Editor + Preview

Edit a file while watching output:

```bash
# Start editor
pilotty spawn vim main.py --name editor

# Start file watcher
pilotty spawn watch -n1 python main.py --name preview

# Edit
pilotty key -s editor i
pilotty type -s editor "print('hello')"
pilotty key -s editor Escape
pilotty type -s editor ":w"
pilotty key -s editor Enter

# Check preview
pilotty snapshot -s preview --format text
```

### Pipeline Workflow

Sequential operations across sessions:

```bash
# Setup
pilotty spawn bash --name worker

# Run commands
pilotty type -s worker "curl -s https://api.example.com > data.json"
pilotty key -s worker Enter
pilotty wait-for -s worker "$"  # Wait for prompt

pilotty type -s worker "jq '.items[]' data.json"
pilotty key -s worker Enter
pilotty wait-for -s worker "$"

# Get output
pilotty snapshot -s worker --format text
```

## Session Isolation

Sessions are fully isolated:
- Separate PTY file descriptors
- Independent screen buffers
- Separate ref numbering per session
- No shared state between sessions

This means:
- `@e1` in session A is unrelated to `@e1` in session B
- Killing session A doesn't affect session B
- Each session can have different terminal sizes

## Daemon Lifecycle

The daemon manages all sessions:

### Auto-Start

The daemon starts automatically on the first command:

```bash
pilotty spawn vim  # Starts daemon if not running
```

### Auto-Stop

After 5 minutes with no active sessions, the daemon shuts down automatically.

### Manual Control

```bash
pilotty daemon     # Manually start daemon
pilotty stop       # Stop daemon and all sessions
```

## Socket Location

The daemon creates a Unix socket at (in priority order):

1. `$PILOTTY_SOCKET_DIR/pilotty.sock`
2. `$XDG_RUNTIME_DIR/pilotty/pilotty.sock`
3. `~/.pilotty/pilotty.sock`
4. `/tmp/pilotty/pilotty.sock`

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PILOTTY_SESSION` | Default session name for all commands |
| `PILOTTY_SOCKET_DIR` | Override socket directory |

Example:

```bash
export PILOTTY_SESSION=editor
pilotty snapshot  # Targets "editor" session without -s flag
```

## Error Handling

### Session Not Found

```json
{
  "code": "SESSION_NOT_FOUND",
  "message": "Session 'myapp' not found",
  "suggestion": "Run 'pilotty list-sessions' to see available sessions"
}
```

### Session Already Exists

Attempting to spawn with a name that's already in use:

```json
{
  "code": "SESSION_EXISTS",
  "message": "Session 'myapp' already exists",
  "suggestion": "Use a different name or kill the existing session first"
}
```

## Best Practices

1. **Use meaningful names**: `--name editor` is better than `--name s1`
2. **Clean up when done**: Kill sessions you're finished with
3. **Don't rely on default**: For multi-session work, always name your sessions
4. **Check session exists**: Use `list-sessions` before targeting
5. **Handle process exit**: Sessions auto-cleanup, but check if your command is still running
