#!/usr/bin/env bash
#
# Syncs the version from Cargo.toml (workspace) to npm/package.json
# Usage: ./scripts/version-sync.sh [--check]
#
# --check: Exit with error if versions are out of sync (useful for CI)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

CARGO_TOML="$ROOT_DIR/Cargo.toml"
PACKAGE_JSON="$ROOT_DIR/npm/package.json"

# Extract version from Cargo.toml workspace section
CARGO_VERSION=$(grep -A5 '^\[workspace\.package\]' "$CARGO_TOML" | grep '^version' | head -1 | sed 's/.*"\(.*\)".*/\1/')

if [ -z "$CARGO_VERSION" ]; then
    echo "ERROR: Could not extract version from $CARGO_TOML"
    exit 1
fi

# Extract current version from package.json
NPM_VERSION=$(grep '"version"' "$PACKAGE_JSON" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

if [ -z "$NPM_VERSION" ]; then
    echo "ERROR: Could not extract version from $PACKAGE_JSON"
    exit 1
fi

echo "Cargo.toml version: $CARGO_VERSION"
echo "package.json version: $NPM_VERSION"

# Check mode: just verify they match
if [ "${1:-}" = "--check" ]; then
    if [ "$CARGO_VERSION" = "$NPM_VERSION" ]; then
        echo "OK: Versions are in sync"
        exit 0
    else
        echo "ERROR: Versions are out of sync!"
        exit 1
    fi
fi

# Sync mode: update package.json if needed
if [ "$CARGO_VERSION" = "$NPM_VERSION" ]; then
    echo "Versions already in sync, nothing to do"
    exit 0
fi

echo "Updating package.json version to $CARGO_VERSION..."

# Use sed to update the version in package.json (portable across macOS/Linux)
if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/\"version\": \"$NPM_VERSION\"/\"version\": \"$CARGO_VERSION\"/" "$PACKAGE_JSON"
else
    sed -i "s/\"version\": \"$NPM_VERSION\"/\"version\": \"$CARGO_VERSION\"/" "$PACKAGE_JSON"
fi

# Verify the update worked
NEW_NPM_VERSION=$(grep '"version"' "$PACKAGE_JSON" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')
if [ "$NEW_NPM_VERSION" = "$CARGO_VERSION" ]; then
    echo "SUCCESS: package.json updated to $CARGO_VERSION"
else
    echo "ERROR: Failed to update package.json"
    exit 1
fi
