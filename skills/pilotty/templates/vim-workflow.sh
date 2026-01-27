#!/bin/bash
# Template: Edit a file with vim
# Creates/edits a file, writes content, saves and exits
#
# Usage: ./vim-workflow.sh <filepath> [content]
# Example: ./vim-workflow.sh /tmp/hello.txt "Hello, World!"

set -euo pipefail

FILE_PATH="${1:?Usage: $0 <filepath> [content]}"
CONTENT="${2:-}"
SESSION_NAME="vim-editor"

echo "Editing: $FILE_PATH"

# 1. Spawn vim with the file
pilotty spawn --name "$SESSION_NAME" vim "$FILE_PATH"

# 2. Wait for vim to be ready (shows filename or new file indicator)
FILENAME=$(basename "$FILE_PATH")
pilotty wait-for -s "$SESSION_NAME" "$FILENAME" -t 10000 || \
  pilotty wait-for -s "$SESSION_NAME" "VIM" -t 5000

echo "Vim ready"

# 3. If content provided, enter insert mode and type it
if [ -n "$CONTENT" ]; then
  echo "Writing content..."
  
  # Enter insert mode
  pilotty key -s "$SESSION_NAME" i
  
  # Type the content
  pilotty type -s "$SESSION_NAME" "$CONTENT"
  
  # Exit insert mode
  pilotty key -s "$SESSION_NAME" Escape
  
  echo "Content written"
fi

# 4. Save and quit
echo "Saving and quitting..."
pilotty type -s "$SESSION_NAME" ":wq"
pilotty key -s "$SESSION_NAME" Enter

# 5. Wait briefly for vim to exit
sleep 0.5

# 6. Check if session is still alive (it shouldn't be)
if pilotty list-sessions 2>/dev/null | grep -q "$SESSION_NAME"; then
  echo "Warning: vim session still active, killing..."
  pilotty kill -s "$SESSION_NAME"
fi

echo "Done. File saved: $FILE_PATH"

# Optionally verify content
if [ -f "$FILE_PATH" ]; then
  echo "--- File contents ---"
  cat "$FILE_PATH"
  echo "--- End ---"
fi
