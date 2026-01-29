# Key Input Reference

Complete reference for key combinations supported by `pilotty key`.

## Basic Usage

```bash
pilotty key <key>                 # Send single key to default session
pilotty key -s myapp <key>        # Send to specific session
pilotty key "key1 key2 key3"      # Send key sequence (space-separated)
pilotty key "key1 key2" --delay 50  # Sequence with 50ms delay between keys
```

## Named Keys

| Key | Aliases | Description |
|-----|---------|-------------|
| `Enter` | `Return` | Enter/Return key |
| `Tab` | | Tab key |
| `Escape` | `Esc` | Escape key |
| `Space` | | Space bar |
| `Backspace` | | Backspace key |
| `Delete` | `Del` | Delete key |
| `Insert` | `Ins` | Insert key |

## Arrow Keys

| Key | Aliases | Description |
|-----|---------|-------------|
| `Up` | `ArrowUp` | Up arrow |
| `Down` | `ArrowDown` | Down arrow |
| `Left` | `ArrowLeft` | Left arrow |
| `Right` | `ArrowRight` | Right arrow |

## Navigation Keys

| Key | Aliases | Description |
|-----|---------|-------------|
| `Home` | | Home key |
| `End` | | End key |
| `PageUp` | `PgUp` | Page up |
| `PageDown` | `PgDn` | Page down |

## Function Keys

| Key | Description |
|-----|-------------|
| `F1` | Function key 1 |
| `F2` | Function key 2 |
| `F3` | Function key 3 |
| `F4` | Function key 4 |
| `F5` | Function key 5 |
| `F6` | Function key 6 |
| `F7` | Function key 7 |
| `F8` | Function key 8 |
| `F9` | Function key 9 |
| `F10` | Function key 10 |
| `F11` | Function key 11 |
| `F12` | Function key 12 |

## Modifier Combinations

### Ctrl Combinations

| Key | Aliases | Common Use |
|-----|---------|------------|
| `Ctrl+C` | `Control+C` | Interrupt/cancel |
| `Ctrl+D` | | EOF/exit |
| `Ctrl+Z` | | Suspend process |
| `Ctrl+L` | | Clear screen |
| `Ctrl+A` | | Beginning of line (bash, emacs) |
| `Ctrl+E` | | End of line (bash, emacs) |
| `Ctrl+K` | | Kill to end of line |
| `Ctrl+U` | | Kill to beginning of line |
| `Ctrl+W` | | Kill word backward |
| `Ctrl+R` | | Reverse search (bash) |
| `Ctrl+S` | | Save (many apps) |
| `Ctrl+Q` | | Quit (some apps) |
| `Ctrl+X` | | Cut / prefix key |
| `Ctrl+V` | | Paste / literal next |
| `Ctrl+G` | | Cancel (emacs) |
| `Ctrl+O` | | Open (many apps) |
| `Ctrl+N` | | Next / new |
| `Ctrl+P` | | Previous |
| `Ctrl+F` | | Forward / find |
| `Ctrl+B` | | Backward |

### Alt Combinations

| Key | Aliases | Common Use |
|-----|---------|------------|
| `Alt+F` | `Meta+F`, `Option+F` | Forward word / File menu |
| `Alt+B` | | Backward word |
| `Alt+D` | | Delete word forward |
| `Alt+Backspace` | | Delete word backward |
| `Alt+.` | | Last argument (bash) |
| `Alt+Tab` | | (Usually handled by window manager) |

### Shift Combinations

| Key | Description |
|-----|-------------|
| `Shift+Tab` | Reverse tab (previous field) |
| `Shift+Enter` | Shift+Enter (app-specific) |
| `Shift+Up` | Select up (some apps) |
| `Shift+Down` | Select down (some apps) |

### Combined Modifiers

| Key | Description |
|-----|-------------|
| `Ctrl+Alt+C` | Ctrl+Alt+C |
| `Ctrl+Shift+C` | Copy (some terminals) |
| `Ctrl+Shift+V` | Paste (some terminals) |

## Special Characters

| Key | Description |
|-----|-------------|
| `Plus` | Literal `+` character |

## Key Sequences

Send multiple keys in order with a single command. Keys are space-separated:

```bash
# Emacs-style chords
pilotty key "Ctrl+X Ctrl+S"       # Save file
pilotty key "Ctrl+X Ctrl+C"       # Exit Emacs
pilotty key "Ctrl+X m"            # Compose mail

# vim command sequences
pilotty key "Escape : w q Enter"  # Save and quit
pilotty key "Escape : q ! Enter"  # Quit without saving
pilotty key "g g d G"             # Delete entire file

# Navigation sequences
pilotty key "Tab Tab Enter"       # Tab twice then Enter
pilotty key "Down Down Space"     # Move down twice and select
```

### Inter-key Delay

Use `--delay` for TUIs that need time between keys:

```bash
pilotty key "Tab Tab Enter" --delay 100   # 100ms between each key
pilotty key "F9 Down Enter" --delay 50    # htop kill menu navigation
```

| Option | Description |
|--------|-------------|
| `--delay <ms>` | Milliseconds between keys (default: 0, max: 10000) |

### When to Use Sequences vs Individual Keys

**Use sequences** for:
- Emacs/vim chords that must be sent together
- Predictable navigation patterns
- Reducing command overhead

**Use individual keys** when:
- You need to check screen state between keys
- Timing is unpredictable
- Different paths based on UI state

## Common TUI Patterns

### Dialog/Whiptail

```bash
pilotty key Tab       # Move between buttons
pilotty key Enter     # Activate button
pilotty key Space     # Toggle checkbox
pilotty key Escape    # Cancel dialog
```

### Vim

```bash
pilotty key i         # Insert mode (use pilotty type for text)
pilotty key Escape    # Normal mode
pilotty key Ctrl+C    # Also exits insert mode
pilotty type ":wq"    # Command (then Enter)
pilotty key Enter

# Using sequences for common operations
pilotty key "Escape : w q Enter"     # Save and quit
pilotty key "Escape : q ! Enter"     # Force quit
pilotty key "Escape d d"             # Delete line
pilotty key "Escape g g"             # Go to top
```

### Htop

```bash
pilotty key F1        # Help
pilotty key F2        # Setup
pilotty key F5        # Tree view
pilotty key F9        # Kill process
pilotty key F10       # Quit
pilotty key q         # Also quit
```

### Less/More

```bash
pilotty key Space     # Page down
pilotty key b         # Page up
pilotty key q         # Quit
pilotty key /         # Search (then type pattern)
pilotty key n         # Next match
pilotty key N         # Previous match
```

### Nano

```bash
pilotty key Ctrl+O    # Save
pilotty key Ctrl+X    # Exit
pilotty key Ctrl+K    # Cut line
pilotty key Ctrl+U    # Paste
pilotty key Ctrl+W    # Search

# Using sequences
pilotty key "Ctrl+O Enter"    # Save with default filename
pilotty key "Ctrl+X n"        # Exit without saving (answer 'n' to save prompt)
```

### Tmux (default prefix)

```bash
pilotty key Ctrl+B    # Prefix key
# Then send the command key:
pilotty key c         # New window
pilotty key n         # Next window
pilotty key p         # Previous window
pilotty key d         # Detach

# Using sequences for tmux commands
pilotty key "Ctrl+B c"    # Prefix + new window
pilotty key "Ctrl+B n"    # Prefix + next window
pilotty key "Ctrl+B d"    # Prefix + detach
```

### Readline/Bash

```bash
pilotty key Ctrl+A    # Beginning of line
pilotty key Ctrl+E    # End of line
pilotty key Ctrl+U    # Clear line
pilotty key Ctrl+R    # Reverse search
pilotty key Ctrl+L    # Clear screen
pilotty key Up        # Previous history
pilotty key Down      # Next history
```

## Case Sensitivity

- Named keys are case-insensitive: `Enter`, `ENTER`, `enter` all work
- Letter keys with Ctrl/Alt are case-insensitive: `Ctrl+c` = `Ctrl+C`
- Plain letters: Use `pilotty type` for text, not `pilotty key`

## Escaping

The `+` character is the modifier separator. To type a literal `+`:

```bash
pilotty key Plus      # Sends the + character
# Or use type for text:
pilotty type "2+2"    # Types "2+2"
```

## Troubleshooting

### Key Not Recognized

```bash
# Check if it's a named key or text
pilotty key Enter     # Named key
pilotty type "hello"  # Text input
```

### Modifier Not Working

Some apps intercept modifiers before the terminal sees them. Try:

```bash
# Check raw terminal behavior
pilotty spawn cat
pilotty key Ctrl+C    # Should show ^C or exit
```

### Timing Issues

Some TUIs need time to process input:

```bash
pilotty key F9        # Opens menu
pilotty wait-for "SIGTERM"  # Wait for menu
pilotty key Enter     # Then select
```
