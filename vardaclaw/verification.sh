#!/bin/bash
set -e

# Configuration
export VARDADB_URL=${VARDADB_URL:-"http://localhost:8000/graphql"}
VARDACLAW_BIN="./target/debug/vardaclaw"
CONFIG_FILE="./config_verification.toml"
TEST_FILE="README.md"

echo "=== VardaClaw Verification ==="
echo "Target URL: $VARDADB_URL"
echo "Binary: $VARDACLAW_BIN"

# Check if binary exists
if [ ! -f "$VARDACLAW_BIN" ]; then
    echo "Error: Binary not found at $VARDACLAW_BIN. Please build first."
    exit 1
fi

# 0. Setup Workspace
export LOCALGPT_WORKSPACE=$(pwd)
export RUST_LOG=debug
echo "Using workspace: $LOCALGPT_WORKSPACE"

# 1. Indexing
echo -e "\n[1] Indexing Workspace (reindex)..."
export LOCALGPT_CONFIG="$CONFIG_FILE"
$VARDACLAW_BIN memory reindex

# 2. Searching
echo -e "\n[2] Searching for 'VardaDB'..."
SEARCH_OUTPUT=$($VARDACLAW_BIN memory search "VardaDB")
echo "$SEARCH_OUTPUT"

# 3. Verification
if echo "$SEARCH_OUTPUT" | grep -q "Found"; then
    echo -e "\n[SUCCESS] Search returned results."
else
    echo -e "\n[FAILURE] Search returned no results."
    exit 1
fi

echo -e "\n=== Verification Complete ==="
