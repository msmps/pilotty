#!/bin/bash
# Element Detection Template
# Demonstrates pilotty's element detection and interaction
#
# Usage: ./element-detection.sh

set -e

# Configuration
PILOTTY="${PILOTTY:-pilotty}"
SESSION="element-demo"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Cleanup on exit
cleanup() {
    $PILOTTY kill -s "$SESSION" 2>/dev/null || true
}
trap cleanup EXIT

echo -e "${BLUE}=== Element Detection Demo ===${NC}"
echo ""

# -----------------------------------------------------------------------------
# Step 1: Spawn a TUI with UI elements
# -----------------------------------------------------------------------------
echo -e "${YELLOW}Step 1: Spawning dialog checklist...${NC}"

$PILOTTY spawn --name "$SESSION" -- dialog --checklist "Select features to enable:" 15 60 5 \
    "notifications" "Push notifications" on \
    "darkmode" "Dark mode theme" off \
    "autosave" "Auto-save documents" on \
    "analytics" "Usage analytics" off \
    "updates" "Auto-updates" on >/dev/null

sleep 0.5

# -----------------------------------------------------------------------------
# Step 2: Get snapshot with elements
# -----------------------------------------------------------------------------
echo -e "${YELLOW}Step 2: Getting snapshot with detected elements...${NC}"
echo ""

SNAPSHOT=$($PILOTTY snapshot -s "$SESSION")

# Show element summary
echo -e "${GREEN}Detected elements:${NC}"
echo "$SNAPSHOT" | jq -r '.elements[] | "  \(.kind) \(.text) at (\(.row),\(.col)) conf=\(.confidence)"'
echo ""

# -----------------------------------------------------------------------------
# Step 3: Analyze toggles
# -----------------------------------------------------------------------------
echo -e "${YELLOW}Step 3: Analyzing toggle states...${NC}"
echo ""

TOGGLES=$(echo "$SNAPSHOT" | jq '[.elements[] | select(.kind == "toggle")]')
CHECKED=$(echo "$TOGGLES" | jq '[.[] | select(.checked == true)] | length')
UNCHECKED=$(echo "$TOGGLES" | jq '[.[] | select(.checked == false)] | length')

echo -e "  Checked toggles:   ${GREEN}$CHECKED${NC}"
echo -e "  Unchecked toggles: ${RED}$UNCHECKED${NC}"
echo ""

# Show each toggle
echo -e "${GREEN}Toggle details:${NC}"
echo "$TOGGLES" | jq -r '.[] | "  \(.text) at (\(.row),\(.col)) checked=\(.checked)"'
echo ""

# -----------------------------------------------------------------------------
# Step 4: Toggle an unchecked option
# -----------------------------------------------------------------------------
echo -e "${YELLOW}Step 4: Toggling 'darkmode' (currently off)...${NC}"

# Get initial hash for change detection
HASH1=$(echo "$SNAPSHOT" | jq -r '.content_hash')

# Navigate to darkmode (second option) and toggle
$PILOTTY key -s "$SESSION" Down >/dev/null  # Move to darkmode
$PILOTTY key -s "$SESSION" Space >/dev/null # Toggle it

sleep 0.2

# Get new snapshot and hash
SNAPSHOT2=$($PILOTTY snapshot -s "$SESSION")
HASH2=$(echo "$SNAPSHOT2" | jq -r '.content_hash')

# Verify change
if [ "$HASH1" != "$HASH2" ]; then
    echo -e "  ${GREEN}Screen changed! (hash: $HASH1 -> $HASH2)${NC}"
else
    echo -e "  ${RED}No change detected${NC}"
fi
echo ""

# Show updated toggle states
echo -e "${GREEN}Updated toggle states:${NC}"
echo "$SNAPSHOT2" | jq -r '.elements[] | select(.kind == "toggle") | "  \(.text) at (\(.row),\(.col)) checked=\(.checked)"'
echo ""

# -----------------------------------------------------------------------------
# Step 5: Find and interact with button
# -----------------------------------------------------------------------------
echo -e "${YELLOW}Step 5: Looking for action button...${NC}"

BUTTON=$(echo "$SNAPSHOT2" | jq -r '.elements[] | select(.kind == "button" or .kind == "input") | "\(.text) at (\(.row),\(.col))"' | head -1)
if [ -n "$BUTTON" ]; then
    echo -e "  Found button: ${GREEN}$BUTTON${NC}"
else
    echo -e "  ${YELLOW}No button element detected, using keyboard to confirm${NC}"
fi
echo ""

# -----------------------------------------------------------------------------
# Step 6: Confirm selection
# -----------------------------------------------------------------------------
echo -e "${YELLOW}Step 6: Confirming selection with Enter...${NC}"

$PILOTTY key -s "$SESSION" Enter >/dev/null

sleep 0.3

# Check final state
echo -e "${GREEN}Final screen state:${NC}"
$PILOTTY snapshot -s "$SESSION" --format text 2>/dev/null | head -5 || echo "  (dialog closed)"
echo ""

# -----------------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------------
echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "This demo showed how to:"
echo "  1. Spawn a TUI application"
echo "  2. Get snapshot with detected elements"
echo "  3. Analyze element states (toggles, buttons)"
echo "  4. Use content_hash for change detection"
echo "  5. Navigate with keyboard based on element context"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
