#!/bin/bash
# Template: Interact with dialog/whiptail prompts
# Demonstrates handling various dialog types with element detection
#
# Usage: ./dialog-interaction.sh
# Requires: dialog or whiptail installed

set -euo pipefail

SESSION_NAME="dialog-demo"

# Check for dialog
if ! command -v dialog &> /dev/null; then
  echo "Error: 'dialog' is not installed"
  echo "Install with: brew install dialog (macOS) or apt install dialog (Linux)"
  exit 1
fi

# Cleanup on exit
cleanup() {
    pilotty kill -s "$SESSION_NAME" 2>/dev/null || true
}
trap cleanup EXIT

echo "=== Dialog Interaction Demo ==="

# --- Yes/No Dialog ---
echo ""
echo "1. Yes/No Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --yesno "Do you want to continue?" 10 40 >/dev/null

# Wait for dialog to render
pilotty wait-for -s "$SESSION_NAME" "continue" -t 5000 >/dev/null

# Show detected elements
echo "Detected elements:"
pilotty snapshot -s "$SESSION_NAME" | jq -r '.elements[] | "  \(.kind) \(.text) at (\(.row),\(.col))"'

# Select Yes using keyboard (Enter selects the default button)
pilotty key -s "$SESSION_NAME" Enter >/dev/null

sleep 0.5
echo "Selected: Yes (via Enter)"

# --- Menu Dialog ---
echo ""
echo "2. Menu Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --menu "Choose an option:" 15 50 4 \
  1 "Option One" \
  2 "Option Two" \
  3 "Option Three" \
  4 "Exit" >/dev/null

pilotty wait-for -s "$SESSION_NAME" "Choose" -t 5000 >/dev/null

# Navigate with arrow keys
pilotty key -s "$SESSION_NAME" Down >/dev/null  # Move to option 2
pilotty key -s "$SESSION_NAME" Down >/dev/null  # Move to option 3
pilotty key -s "$SESSION_NAME" Enter >/dev/null # Select

sleep 0.5
echo "Selected: Option Three (via arrow keys + Enter)"

# --- Checklist Dialog with Element Detection ---
echo ""
echo "3. Checklist Dialog (with element detection)"

pilotty spawn --name "$SESSION_NAME" dialog --checklist "Select items:" 15 50 4 \
  1 "Item A" off \
  2 "Item B" off \
  3 "Item C" off \
  4 "Item D" off >/dev/null

pilotty wait-for -s "$SESSION_NAME" "Select" -t 5000 >/dev/null

# Show initial toggle states
echo "Initial toggle states:"
pilotty snapshot -s "$SESSION_NAME" | jq -r '.elements[] | select(.kind == "toggle") | "  \(.text) at (\(.row),\(.col)) checked=\(.checked)"'

# Toggle items with Space
pilotty key -s "$SESSION_NAME" Space >/dev/null      # Toggle Item A
pilotty key -s "$SESSION_NAME" Down >/dev/null
pilotty key -s "$SESSION_NAME" Down >/dev/null
pilotty key -s "$SESSION_NAME" Space >/dev/null      # Toggle Item C

# Show updated toggle states
echo "After toggling:"
pilotty snapshot -s "$SESSION_NAME" | jq -r '.elements[] | select(.kind == "toggle") | "  \(.text) at (\(.row),\(.col)) checked=\(.checked)"'

pilotty key -s "$SESSION_NAME" Enter >/dev/null      # Confirm

sleep 0.5
echo "Selected: Item A, Item C"

# --- Input Dialog ---
echo ""
echo "4. Input Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --inputbox "Enter your name:" 10 40 >/dev/null

pilotty wait-for -s "$SESSION_NAME" "name" -t 5000 >/dev/null

# Show detected input element
echo "Detected input element:"
pilotty snapshot -s "$SESSION_NAME" | jq -r '.elements[] | select(.kind == "input") | "  \(.kind) at (\(.row),\(.col)) width=\(.width)"'

# Type input
pilotty type -s "$SESSION_NAME" "Agent Smith"
pilotty key -s "$SESSION_NAME" Enter >/dev/null

sleep 0.5
echo "Entered: Agent Smith"

# --- Message Box (final) ---
echo ""
echo "5. Message Box"

pilotty spawn --name "$SESSION_NAME" dialog --msgbox "Demo complete!" 10 40 >/dev/null

pilotty wait-for -s "$SESSION_NAME" "complete" -t 5000 >/dev/null

# Show button element
echo "Detected button:"
pilotty snapshot -s "$SESSION_NAME" | jq -r '.elements[] | select(.kind == "button" or .kind == "input") | "  \(.kind) \(.text) at (\(.row),\(.col))"'

# Dismiss with Enter
pilotty key -s "$SESSION_NAME" Enter >/dev/null

sleep 0.5

echo ""
echo "=== Demo Complete ==="
echo ""
echo "Key takeaways:"
echo "  - Use snapshot | jq '.elements' to see detected UI elements"
echo "  - Toggles have 'checked' field for state tracking"
echo "  - Use keyboard (Tab, Space, Enter, arrows) for reliable navigation"
echo "  - content_hash can detect screen changes between snapshots"
