#!/bin/bash
# Tokio Console Diagnostic Script
# Captures tokio-console state from a remote server via SSH
# Usage: ./scripts/diagnose_tokio_console.sh <server_name>
# Example: ./scripts/diagnose_tokio_console.sh communities

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Configuration
SERVER="${1:-communities}"
OUTPUT_DIR="${OUTPUT_DIR:-.}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
OUTPUT_FILE="$OUTPUT_DIR/tokio-console-$SERVER-$TIMESTAMP.txt"

echo "======================================"
echo "Tokio Console Remote Diagnostic"
echo "======================================"
echo "Server: $SERVER"
echo "Output: $OUTPUT_FILE"
echo ""

# Test SSH connection
echo "Testing SSH connection to $SERVER..."
if ! ssh -o ConnectTimeout=5 "$SERVER" "echo connected" &>/dev/null; then
    echo -e "${RED}Error: Cannot SSH to $SERVER${NC}"
    echo "Make sure SSH is configured (try: ssh $SERVER)"
    exit 1
fi
echo -e "${GREEN}✓ SSH connection OK${NC}"

# Check if tokio-console is available on remote
echo "Checking tokio-console on $SERVER..."
if ! ssh "$SERVER" "source ~/.cargo/env 2>/dev/null && which tokio-console" &>/dev/null; then
    echo -e "${RED}Error: tokio-console not found on $SERVER${NC}"
    echo "Install with: ssh $SERVER 'source ~/.cargo/env && cargo install --locked tokio-console'"
    exit 1
fi
echo -e "${GREEN}✓ tokio-console found${NC}"

# Check if port 6669 is reachable
echo "Checking console-subscriber on $SERVER:6669..."
if ! ssh "$SERVER" "timeout 2 bash -c '</dev/tcp/localhost/6669' 2>/dev/null"; then
    echo -e "${RED}Error: Cannot connect to console-subscriber on $SERVER${NC}"
    echo "Make sure the groups_relay container is running with console feature enabled"
    exit 1
fi
echo -e "${GREEN}✓ console-subscriber reachable${NC}"
echo ""

# Capture tokio-console state
echo "Capturing tokio-console state (waiting 5s for data)..."
SESSION="tokio_diag_$$"

# Run tokio-console via SSH in tmux, capture, and cleanup
ssh "$SERVER" bash <<EOF
set -e
source ~/.cargo/env 2>/dev/null || true

# Start tokio-console in background tmux session
tmux new-session -d -s "$SESSION" "tokio-console http://localhost:6669" 2>/dev/null || {
    echo "Error: Failed to start tmux session" >&2
    exit 1
}

# Wait for console to connect and gather data
sleep 5

# Capture the pane content
tmux capture-pane -t "$SESSION" -p -S - 2>/dev/null || {
    tmux kill-session -t "$SESSION" 2>/dev/null
    echo "Error: Failed to capture pane" >&2
    exit 1
}

# Cleanup
tmux kill-session -t "$SESSION" 2>/dev/null
EOF

if [ $? -eq 0 ]; then
    # Save the output locally
    ssh "$SERVER" bash <<EOF | tee "$OUTPUT_FILE"
source ~/.cargo/env 2>/dev/null || true
tmux new-session -d -s "$SESSION" "tokio-console http://localhost:6669" 2>/dev/null
sleep 5
tmux capture-pane -t "$SESSION" -p -S -
tmux kill-session -t "$SESSION" 2>/dev/null
EOF

    echo ""
    echo -e "${GREEN}✓ Snapshot captured successfully${NC}"
    echo ""
    echo "Output saved to: $OUTPUT_FILE"
    echo ""

    # Show summary
    echo "======================================"
    echo "Summary"
    echo "======================================"

    # Extract key information
    if grep -q "tasks have lost their wakers" "$OUTPUT_FILE"; then
        lost_wakers=$(grep -o "[0-9]* tasks have lost their wakers" "$OUTPUT_FILE" | head -1)
        echo -e "${YELLOW}⚠  $lost_wakers${NC}"
    fi

    if grep -q "tasks are.*bytes or larger" "$OUTPUT_FILE"; then
        large_tasks=$(grep -o "[0-9]* tasks are [0-9]* bytes or larger" "$OUTPUT_FILE" | head -1)
        echo -e "${YELLOW}⚠  $large_tasks${NC}"
    fi

    # Count total tasks
    if grep -q "Tasks ([0-9]*)" "$OUTPUT_FILE"; then
        total_tasks=$(grep -o "Tasks ([0-9]*)" "$OUTPUT_FILE" | head -1 | grep -o "[0-9]*")
        running_tasks=$(grep -o "Running ([0-9]*)" "$OUTPUT_FILE" | head -1 | grep -o "[0-9]*")
        idle_tasks=$(grep -o "Idle ([0-9]*)" "$OUTPUT_FILE" | head -1 | grep -o "[0-9]*")

        echo "Total tasks: $total_tasks"
        echo "  Running: $running_tasks"
        echo "  Idle: $idle_tasks"
    fi

    echo ""
    echo "To view the full snapshot:"
    echo "  cat $OUTPUT_FILE"
    echo ""
    echo "  Or with colors:"
    echo "  less -R $OUTPUT_FILE"

else
    echo -e "${RED}Error: Failed to capture tokio-console snapshot${NC}"
    echo ""
    echo "You can manually connect with:"
    echo "  ssh $SERVER"
    echo "  source ~/.cargo/env"
    echo "  tokio-console http://localhost:6669"
    exit 1
fi

echo ""
echo "======================================"
echo "Common Next Steps"
echo "======================================"
echo ""
echo "1. Review the snapshot for warning indicators (⚠)"
echo "2. Look for tasks with high poll counts"
echo "3. Check for tasks with long total time but low busy time"
echo "4. See docs/debugging_async_issues.md for detailed guidance"
echo ""
