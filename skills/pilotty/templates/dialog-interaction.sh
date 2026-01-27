#!/bin/bash
# Template: Interact with dialog/whiptail prompts
# Demonstrates handling various dialog types
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

echo "=== Dialog Interaction Demo ==="

# --- Yes/No Dialog ---
echo ""
echo "1. Yes/No Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --yesno "Do you want to continue?" 10 40

# Wait for dialog to render
pilotty wait-for -s "$SESSION_NAME" "continue" -t 5000

# Take snapshot to see buttons
echo "Snapshot:"
pilotty snapshot -s "$SESSION_NAME" --format compact

# Select Yes using keyboard (Enter selects the default button)
pilotty key -s "$SESSION_NAME" Enter  # Select default (Yes)

sleep 0.5
echo "Selected: Yes"

# --- Menu Dialog ---
echo ""
echo "2. Menu Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --menu "Choose an option:" 15 50 4 \
  1 "Option One" \
  2 "Option Two" \
  3 "Option Three" \
  4 "Exit"

pilotty wait-for -s "$SESSION_NAME" "Choose" -t 5000

# Navigate with arrow keys (pilotty auto-detects application cursor mode)
pilotty key -s "$SESSION_NAME" Down  # Move to option 2
pilotty key -s "$SESSION_NAME" Down  # Move to option 3
pilotty key -s "$SESSION_NAME" Enter # Select

sleep 0.5
echo "Selected: Option Three"

# --- Checklist Dialog ---
echo ""
echo "3. Checklist Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --checklist "Select items:" 15 50 4 \
  1 "Item A" off \
  2 "Item B" off \
  3 "Item C" off \
  4 "Item D" off

pilotty wait-for -s "$SESSION_NAME" "Select" -t 5000

# Toggle items with Space
pilotty key -s "$SESSION_NAME" Space      # Toggle Item A
pilotty key -s "$SESSION_NAME" Down
pilotty key -s "$SESSION_NAME" Down
pilotty key -s "$SESSION_NAME" Space      # Toggle Item C
pilotty key -s "$SESSION_NAME" Enter      # Confirm

sleep 0.5
echo "Selected: Item A, Item C"

# --- Input Dialog ---
echo ""
echo "4. Input Dialog"

pilotty spawn --name "$SESSION_NAME" dialog --inputbox "Enter your name:" 10 40

pilotty wait-for -s "$SESSION_NAME" "name" -t 5000

# Type input
pilotty type -s "$SESSION_NAME" "Agent Smith"
pilotty key -s "$SESSION_NAME" Enter

sleep 0.5
echo "Entered: Agent Smith"

# --- Message Box (final) ---
echo ""
echo "5. Message Box"

pilotty spawn --name "$SESSION_NAME" dialog --msgbox "Demo complete!" 10 40

pilotty wait-for -s "$SESSION_NAME" "complete" -t 5000

# Take final snapshot to see the OK button
pilotty snapshot -s "$SESSION_NAME"

# Dismiss with Enter
pilotty key -s "$SESSION_NAME" Enter

sleep 0.5

# Cleanup
if pilotty list-sessions 2>/dev/null | grep -q "$SESSION_NAME"; then
  pilotty kill -s "$SESSION_NAME"
fi

echo ""
echo "=== Demo Complete ==="
