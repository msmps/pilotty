# Region Refs Workflow

pilotty assigns stable references (`@e1`, `@e2`, etc.) to interactive elements detected in terminal output. This enables reliable element targeting without fragile coordinate-based clicking.

## How It Works

### The Problem

Traditional terminal automation uses coordinates:
```
Click at (10, 5) → Hope button is still there → Often breaks
```

### The Solution

pilotty detects regions and assigns refs:
```
Snapshot → Regions with @refs → Click by ref → Reliable
```

## The Snapshot Command

```bash
# Full JSON output (default)
pilotty snapshot

# Compact format (smaller, refs inline)
pilotty snapshot --format compact

# Plain text only (for human reading)
pilotty snapshot --format text
```

## Snapshot Output Format

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
  "text": "... full screen text ..."
}
```

## Using Refs

Once you have refs, interact directly:

```bash
# Click the OK button
pilotty click @e1

# Or if targeting a different session
pilotty click -s myapp @e2
```

## Ref Lifecycle

**Critical**: Refs are invalidated when the screen changes significantly!

```bash
# Get initial snapshot
pilotty snapshot
# @e1 [button] "[ Next ]"

# Click triggers screen change
pilotty click @e1

# MUST re-snapshot to get new refs
pilotty snapshot
# @e1 [button] "[ Back ]"  <- Different element now!
```

## Ref Stability

Refs are designed to be stable when content is similar:

| Scenario | Ref Behavior |
|----------|--------------|
| Same element, same position | Ref preserved |
| Same element, slight movement | Ref usually preserved |
| Element content changed | New ref assigned |
| Element disappeared | Ref invalid, click fails |
| New element appeared | New ref assigned |

## Best Practices

### 1. Always Snapshot Before Interacting

```bash
# CORRECT
pilotty spawn dialog --msgbox "Hello" 10 40
pilotty wait-for "Hello"
pilotty snapshot             # Get refs first
pilotty click @e1            # Use ref

# WRONG
pilotty spawn dialog --msgbox "Hello" 10 40
pilotty click @e1            # Ref doesn't exist yet!
```

### 2. Re-Snapshot After Screen Changes

```bash
pilotty click @e1            # Opens new dialog
pilotty snapshot             # Get new refs for new screen
pilotty click @e1            # Click element in new dialog
```

### 3. Re-Snapshot After Dynamic Updates

```bash
pilotty key Down             # Move selection in menu
pilotty snapshot             # See updated highlight
pilotty click @e3            # Click new position
```

### 4. Use wait-for Before Snapshot

```bash
pilotty spawn my-app
pilotty wait-for "Ready"     # Ensure app is stable
pilotty snapshot             # Now snapshot is reliable
```

## Region Detection Details

pilotty detects regions via:

1. **Box characters**: `+-|` and Unicode box-drawing (`┌─┐│└┘` etc.)
2. **Inverse video**: SGR attribute 7 (highlighted/selected items)
3. **Colored backgrounds**: Non-default background colors
4. **Underlines**: Text decoration (often links)
5. **Bracket patterns**: `[ OK ]`, `< Save >`, `[x] Option`

Detection is conservative: better to miss an element than hallucinate one.

### Detection Patterns

| Pattern | Detected As | Example |
|---------|-------------|---------|
| `[ text ]` | button | `[ OK ]`, `[ Cancel ]` |
| `< text >` | button | `< Yes >`, `< No >` |
| `[x]` or `[ ]` | checkbox | `[x] Enable`, `[ ] Disable` |
| `(*)` or `( )` | radio_button | `(*) Option A`, `( ) Option B` |
| `(X)text` | menu_item | `(F)ile`, `(E)dit` |
| Inverse video block | menu_item | Highlighted menu selection |
| Underlined text | link | URLs, references |
| Box-drawn region | text_input or unknown | Input fields, dialogs |

## Troubleshooting

### "Ref not found" Error

The ref was invalidated. Re-snapshot:

```bash
pilotty snapshot
# Find the new ref for your target element
pilotty click @e2  # Use updated ref
```

### Element Not in Snapshot

The element may be:
- Off-screen: scroll to reveal it
- Not yet rendered: wait longer
- Not detected: use coordinates or keys instead

```bash
# Scroll to reveal
pilotty scroll down 10
pilotty snapshot

# Or wait for render
pilotty wait-for "Submit"
pilotty snapshot
```

### Too Many/Few Regions

Region detection depends on the TUI's rendering. If detection is imperfect:

```bash
# Use text output to see raw screen
pilotty snapshot --format text

# Use keyboard navigation instead
pilotty key Tab      # Move to next element
pilotty key Enter    # Activate
```

### Wrong Element Clicked

Verify you're clicking the intended ref:

```bash
pilotty snapshot
# Check output: @e1 is "[ OK ]" at (10, 5)
# If @e1 is wrong element, find correct ref in output
```

## Ref Notation

```
@e1 [button] "[ OK ]"
│    │        │
│    │        └─ Element text/content
│    └─ Region type
└─ Unique ref ID
```

Ref IDs are sequential (`@e1`, `@e2`, `@e3`...) and reset when the session's region tracker detects major screen changes.
