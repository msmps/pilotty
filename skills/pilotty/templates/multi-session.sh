#!/bin/bash
# Template: Multi-session orchestration
# Run multiple TUI apps in parallel and interact with each
#
# Usage: ./multi-session.sh
# Demonstrates parallel session management

set -euo pipefail

echo "=== Multi-Session Orchestration Demo ==="

# Session names
SESSION_SHELL="worker-shell"
SESSION_MONITOR="system-monitor"
SESSION_EDITOR="file-editor"

cleanup() {
  echo ""
  echo "Cleaning up sessions..."
  pilotty kill -s "$SESSION_SHELL" 2>/dev/null || true
  pilotty kill -s "$SESSION_MONITOR" 2>/dev/null || true
  pilotty kill -s "$SESSION_EDITOR" 2>/dev/null || true
  echo "Done."
}

trap cleanup EXIT

# --- 1. Start all sessions ---
echo ""
echo "1. Starting sessions..."

# Shell for running commands
pilotty spawn --name "$SESSION_SHELL" bash
echo "   Started: $SESSION_SHELL (bash)"

# System monitor (top is more portable than htop)
pilotty spawn --name "$SESSION_MONITOR" top
echo "   Started: $SESSION_MONITOR (top)"

# Editor for a temp file
TEMP_FILE="/tmp/pilotty-demo-$$.txt"
pilotty spawn --name "$SESSION_EDITOR" vi "$TEMP_FILE"
echo "   Started: $SESSION_EDITOR (vi)"

# Wait for all to be ready
pilotty wait-for -s "$SESSION_SHELL" '$' -t 5000 || true
pilotty wait-for -s "$SESSION_MONITOR" "load" -t 5000 || pilotty wait-for -s "$SESSION_MONITOR" "CPU" -t 5000 || true
pilotty wait-for -s "$SESSION_EDITOR" "~" -t 5000 || true

echo "   All sessions ready"

# --- 2. List active sessions ---
echo ""
echo "2. Active sessions:"
pilotty list-sessions

# --- 3. Interact with shell ---
echo ""
echo "3. Running command in shell session..."

pilotty type -s "$SESSION_SHELL" 'echo "Hello from pilotty multi-session demo"'
pilotty key -s "$SESSION_SHELL" Enter

# Wait for command to complete
sleep 0.5
pilotty wait-for -s "$SESSION_SHELL" '$' -t 5000

# Capture output
echo "   Shell output:"
pilotty snapshot -s "$SESSION_SHELL" --format text | tail -5

# --- 4. Check monitor ---
echo ""
echo "4. Checking system monitor..."

pilotty snapshot -s "$SESSION_MONITOR" --format text | head -10
echo "   (truncated)"

# --- 5. Write to editor ---
echo ""
echo "5. Writing to editor..."

# Enter insert mode
pilotty key -s "$SESSION_EDITOR" i

# Type content
pilotty type -s "$SESSION_EDITOR" "# Multi-session demo"
pilotty key -s "$SESSION_EDITOR" Enter
pilotty type -s "$SESSION_EDITOR" "This file was created by pilotty"
pilotty key -s "$SESSION_EDITOR" Enter
pilotty type -s "$SESSION_EDITOR" "Running $(date)"

# Exit insert mode
pilotty key -s "$SESSION_EDITOR" Escape

# Save (but don't quit yet)
pilotty type -s "$SESSION_EDITOR" ":w"
pilotty key -s "$SESSION_EDITOR" Enter

echo "   Content written to $TEMP_FILE"

# --- 6. Run another shell command ---
echo ""
echo "6. Running another shell command..."

pilotty type -s "$SESSION_SHELL" "cat $TEMP_FILE"
pilotty key -s "$SESSION_SHELL" Enter

sleep 0.5
pilotty wait-for -s "$SESSION_SHELL" '$' -t 5000

echo "   File contents from shell:"
pilotty snapshot -s "$SESSION_SHELL" --format text | grep -A5 "Multi-session" || true

# --- 7. Close editor ---
echo ""
echo "7. Closing editor..."

pilotty type -s "$SESSION_EDITOR" ":q"
pilotty key -s "$SESSION_EDITOR" Enter

sleep 0.5

# --- 8. Stop monitor ---
echo ""
echo "8. Stopping monitor..."

pilotty key -s "$SESSION_MONITOR" q

sleep 0.5

# --- 9. Final shell command ---
echo ""
echo "9. Final shell command..."

pilotty type -s "$SESSION_SHELL" "echo 'Demo complete!'"
pilotty key -s "$SESSION_SHELL" Enter

sleep 0.5

# --- Summary ---
echo ""
echo "=== Demo Summary ==="
echo "Demonstrated:"
echo "  - Starting multiple named sessions"
echo "  - Interacting with each independently"
echo "  - Running commands in a shell session"
echo "  - Monitoring system with top"
echo "  - Editing files with vi"
echo "  - Capturing output from sessions"
echo ""
echo "Sessions will be cleaned up on exit."

# Cleanup handled by trap
