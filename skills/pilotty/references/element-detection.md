# Element Detection

pilotty automatically detects interactive UI elements in terminal applications. Elements provide **read-only context** to help agents understand UI structure.

## Overview

pilotty analyzes terminal screen content and detects:
- **Toggles**: Checkboxes like `[x]`, `[ ]`, `[*]`, `☑`, `☐`
- **Buttons**: Action elements like `[OK]`, `<Cancel>`, `(Submit)`
- **Inputs**: Text fields marked by underscores `____` or cursor position

Each detected element includes:
- Kind, position (row, col), width, text content
- Confidence score (0.0-1.0)
- State information (checked for toggles, focused for inputs/buttons)

## Detection Rules

### Priority Order (Highest to Lowest)

1. **Cursor Position** - Input (confidence: 1.0, focused: true)
2. **Checkbox Patterns** - Toggle (confidence: 1.0)
3. **Inverse Video** - Button (confidence: 1.0, focused: true)
4. **Bracket Patterns** - Button (confidence: 0.8)
5. **Underscore Fields** - Input (confidence: 0.6)

### Toggle Detection

Toggles are detected from checkbox patterns:

| Pattern | State | Notes |
|---------|-------|-------|
| `[x]`, `[X]` | checked: true | Standard checked |
| `[ ]` | checked: false | Standard unchecked |
| `[*]` | checked: true | Dialog/ncurses style |
| `☑`, `✓`, `✔`, `☒` | checked: true | Unicode checkmarks |
| `☐`, `□` | checked: false | Unicode unchecked |

Example detection:
```json
{
  "kind": "toggle",
  "row": 5,
  "col": 2,
  "width": 3,
  "text": "[x]",
  "confidence": 1.0,
  "checked": true
}
```

### Button Detection

Buttons are detected from:

1. **Inverse video** (highest confidence)
   - Text with reversed foreground/background colors
   - Common in dialog, whiptail, and ncurses apps
   - Confidence: 1.0, focused: true

2. **Bracket patterns** (medium confidence)
   - Square brackets: `[OK]`, `[Cancel]`, `[Save]`
   - Angle brackets: `<Yes>`, `<No>`
   - Parentheses: `(Submit)`, `(Reset)`
   - Confidence: 0.8

Example detection:
```json
{
  "kind": "button",
  "row": 10,
  "col": 5,
  "width": 6,
  "text": "[Save]",
  "confidence": 0.8
}
```

### Input Detection

Inputs are detected from:

1. **Cursor position** (highest confidence)
   - The cell where the cursor is located
   - Confidence: 1.0, focused: true

2. **Underscore runs** (lower confidence)
   - 3+ consecutive underscores: `___`, `__________`
   - Common in form-style TUIs
   - Confidence: 0.6

Example detection:
```json
{
  "kind": "input",
  "row": 8,
  "col": 12,
  "width": 10,
  "text": "__________",
  "confidence": 0.6
}
```

## Non-Interactive Patterns (Filtered)

The following patterns are recognized but NOT returned as interactive elements:

| Pattern | Why Filtered |
|---------|--------------|
| `http://`, `https://` | Links are not clickable in most TUIs |
| `[====]`, `[####]` | Progress bars |
| `[ERROR]`, `[WARNING]`, `[INFO]` | Status indicators |
| `[1]`, `[2]`, `1)`, `a)` | Menu prefixes |
| `├`, `┤`, `│`, `┌`, `┐` | Box-drawing characters |

## Element Fields Reference

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `kind` | string | Yes | `button`, `input`, or `toggle` |
| `row` | number | Yes | Row position (0-based from top) |
| `col` | number | Yes | Column position (0-based from left) |
| `width` | number | Yes | Width in terminal cells |
| `text` | string | Yes | Element text content |
| `confidence` | number | Yes | Detection confidence (0.0-1.0) |
| `focused` | bool | No | Present and true if element has focus |
| `checked` | bool | No | Present for toggles only |

### Width Calculation

Element width uses Unicode display width:
- ASCII characters: width 1
- CJK characters (Chinese, Japanese, Korean): width 2
- Emoji: width 2
- Zero-width characters: width 0

This matches terminal column alignment.

## Content Hash

Each snapshot includes a `content_hash` field for change detection:

```json
{
  "content_hash": 12345678901234567890,
  ...
}
```

The hash is computed from the visible screen text content. Use it to:
- Detect if the screen changed between snapshots
- Avoid re-processing unchanged screens

```bash
HASH1=$(pilotty snapshot | jq -r '.content_hash')
pilotty key Tab
HASH2=$(pilotty snapshot | jq -r '.content_hash')
[ "$HASH1" != "$HASH2" ] && echo "Screen changed"
```

## Best Practices

### 1. Elements for Understanding, Keyboard for Interaction

Elements tell you WHAT is on screen. Use keyboard to interact:

```bash
# See what's on screen
pilotty snapshot | jq '.elements[] | {kind, text, row, col, checked}'

# Navigate with keyboard
pilotty key Tab    # Move between elements
pilotty key Space  # Toggle checkboxes
pilotty key Enter  # Activate buttons
```

### 2. Check Confidence Levels

Higher confidence means more reliable detection:

```bash
# Filter to high-confidence elements only
pilotty snapshot | jq '.elements[] | select(.confidence >= 0.8)'
```

### 3. Find Elements by Content or Position

```bash
# Find element by text content
pilotty snapshot | jq '.elements[] | select(.text | contains("Save"))'

# Find element at specific position
pilotty snapshot | jq '.elements[] | select(.row == 5 and .col == 10)'

# Get first toggle
pilotty snapshot | jq '[.elements[] | select(.kind == "toggle")][0]'
```

## Limitations

### What Detection Does NOT Find

1. **Menu items without markers** - Plain text menus need keyboard navigation
2. **Custom widgets** - Non-standard UI patterns may not be recognized
3. **Color-only highlighting** - Elements must have text patterns or inverse video
4. **Disabled elements** - No distinction between enabled/disabled

### What Detection Cannot Do

1. **Click elements directly by name** - Use row/col with click command
2. **Track elements across screens** - Elements may move; use text content to re-find

## Troubleshooting

### No Elements Detected

1. Check if the app uses standard patterns:
   ```bash
   pilotty snapshot --format text  # View raw screen
   ```

2. Look for inverse video (may show elements on button/input):
   ```bash
   pilotty snapshot | jq '.elements[] | select(.confidence == 1.0)'
   ```

### Wrong Element Kind

The classifier uses heuristics. If `[x]` is detected as a button instead of toggle:
1. Check for surrounding context
2. Use `text` field to identify element purpose

### Elements Missing After Action

Element positions may change between snapshots. Track elements by:
- Text content (most reliable)
- Element kind
- Approximate row/column position

## Example: Complete Workflow

```bash
#!/bin/bash
SESSION="form"

# 1. Spawn application
pilotty spawn --name $SESSION dialog --checklist "Options:" 15 50 4 \
    "opt1" "Feature A" on \
    "opt2" "Feature B" off \
    "opt3" "Feature C" on \
    "opt4" "Feature D" off

sleep 0.5

# 2. Analyze initial state
echo "Initial state:"
pilotty snapshot -s $SESSION | jq '.elements[] | select(.kind == "toggle") | {text, checked}'

# 3. Find unchecked toggles
UNCHECKED=$(pilotty snapshot -s $SESSION | jq '[.elements[] | select(.kind == "toggle" and .checked == false)] | length')
echo "Unchecked toggles: $UNCHECKED"

# 4. Navigate and toggle opt2
pilotty key -s $SESSION Down   # Move to opt2
pilotty key -s $SESSION Space  # Toggle it

# 5. Verify change via content_hash
HASH1=$(pilotty snapshot -s $SESSION | jq -r '.content_hash')
echo "Hash after toggle: $HASH1"

# 6. Confirm and check final state
pilotty key -s $SESSION Enter
sleep 0.3

echo "Final state:"
pilotty snapshot -s $SESSION | jq '.elements[] | select(.kind == "toggle") | {text, checked}'

# 7. Cleanup
pilotty kill -s $SESSION
```
