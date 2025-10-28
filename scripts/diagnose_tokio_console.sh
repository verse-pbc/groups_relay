#!/bin/bash
# Tokio Console Diagnostic Script
# Captures current tokio-console state for debugging async issues

set -e

# Configuration
TOKIO_CONSOLE_ADDR="${TOKIO_CONSOLE_ADDR:-http://localhost:6669}"
OUTPUT_DIR="${OUTPUT_DIR:-$HOME}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
OUTPUT_FILE="$OUTPUT_DIR/tokio-console-snapshot-$TIMESTAMP.txt"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "======================================"
echo "Tokio Console Diagnostic Tool"
echo "======================================"
echo ""

# Check if tokio-console is installed
if ! command -v tokio-console &> /dev/null; then
    echo -e "${RED}Error: tokio-console not found${NC}"
    echo "Install with: cargo install --locked tokio-console"
    echo "Or: source ~/.cargo/env (if already installed)"
    exit 1
fi

echo -e "${GREEN}✓ tokio-console found${NC}"

# Check if server is reachable
echo "Checking connection to $TOKIO_CONSOLE_ADDR..."
if ! timeout 2 bash -c "</dev/tcp/localhost/6669" 2>/dev/null; then
    echo -e "${RED}Error: Cannot connect to $TOKIO_CONSOLE_ADDR${NC}"
    echo "Make sure the server is running with the 'console' feature enabled"
    exit 1
fi

echo -e "${GREEN}✓ Server is reachable${NC}"
echo ""

# Function to capture console state
capture_console_state() {
    local session_name="tokio_diagnosis_$$"
    local duration=${1:-5}

    echo "Capturing tokio-console state (waiting ${duration}s for data)..."

    # Start tokio-console in tmux
    if command -v tmux &> /dev/null; then
        # Using tmux
        tmux new-session -d -s "$session_name" "source ~/.cargo/env 2>/dev/null; tokio-console $TOKIO_CONSOLE_ADDR" || {
            # tmux not available or failed, try without tmux
            echo -e "${YELLOW}Warning: tmux not available, using alternative method${NC}"
            return 1
        }

        # Wait for console to connect and gather data
        sleep "$duration"

        # Capture the pane content
        tmux capture-pane -t "$session_name" -p -S - > "$OUTPUT_FILE" 2>/dev/null || {
            echo -e "${RED}Error: Failed to capture pane${NC}"
            tmux kill-session -t "$session_name" 2>/dev/null
            return 1
        }

        # Cleanup
        tmux kill-session -t "$session_name" 2>/dev/null

        return 0
    else
        echo -e "${YELLOW}Warning: tmux not available${NC}"
        return 1
    fi
}

# Capture the state
if capture_console_state 5; then
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
        echo -e "${YELLOW}⚠ Warning: $lost_wakers${NC}"
    fi

    if grep -q "tasks are.*bytes or larger" "$OUTPUT_FILE"; then
        large_tasks=$(grep -o "[0-9]* tasks are [0-9]* bytes or larger" "$OUTPUT_FILE" | head -1)
        echo -e "${YELLOW}⚠ Warning: $large_tasks${NC}"
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
    echo "To view in less:"
    echo "  less -R $OUTPUT_FILE"

else
    echo -e "${YELLOW}Warning: Could not auto-capture snapshot${NC}"
    echo ""
    echo "You can manually connect with:"
    echo "  tokio-console $TOKIO_CONSOLE_ADDR"
    echo ""
    echo "Or capture manually with tmux:"
    echo "  tmux new-session -d -s diag 'tokio-console $TOKIO_CONSOLE_ADDR'"
    echo "  sleep 5"
    echo "  tmux capture-pane -t diag -p -S - > snapshot.txt"
    echo "  tmux kill-session -t diag"
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
