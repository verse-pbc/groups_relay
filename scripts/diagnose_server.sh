#!/bin/bash
# Comprehensive Server Diagnostic Script
# Captures all relevant metrics for diagnosing server hangs
# Usage: ./scripts/diagnose_server.sh <server_name>
# Example: ./scripts/diagnose_server.sh communities

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
SERVER="${1:-communities}"
CONTAINER_NAME="groups_relay"
OUTPUT_DIR="${OUTPUT_DIR:-./diagnostics}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
OUTPUT_FILE="$OUTPUT_DIR/diagnostic-$SERVER-$TIMESTAMP.txt"

mkdir -p "$OUTPUT_DIR"

echo "======================================"
echo "Comprehensive Server Diagnostic"
echo "======================================"
echo "Server: $SERVER"
echo "Container: $CONTAINER_NAME"
echo "Output: $OUTPUT_FILE"
echo ""

# Test SSH connection
echo "Testing SSH connection to $SERVER..."
if ! ssh -o ConnectTimeout=5 "$SERVER" "echo connected" &>/dev/null; then
    echo -e "${RED}Error: Cannot SSH to $SERVER${NC}"
    exit 1
fi
echo -e "${GREEN}✓ SSH connection OK${NC}"
echo ""

# Helper function to add section headers
section() {
    {
        echo ""
        echo "========================================"
        echo "$1"
        echo "========================================"
    } | tee -a "$OUTPUT_FILE"
}

# Start diagnostic collection
{
    echo "Diagnostic Report for $SERVER"
    echo "Generated: $(date)"
    echo "Hostname: $(ssh "$SERVER" hostname)"
} > "$OUTPUT_FILE"

# 1. Docker Container State
section "Docker Container State"
echo -e "${BLUE}Checking container status...${NC}"
ssh "$SERVER" "docker ps -f name=$CONTAINER_NAME --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}'" | tee -a "$OUTPUT_FILE"
echo "" | tee -a "$OUTPUT_FILE"
ssh "$SERVER" "docker inspect $CONTAINER_NAME --format='Restart Count: {{.RestartCount}}
OOMKilled: {{.State.OOMKilled}}
Health Status: {{.State.Health.Status}}
Memory Limit: {{.HostConfig.Memory}}
PID: {{.State.Pid}}'" | tee -a "$OUTPUT_FILE"

# 2. Resource Usage (docker stats)
section "Resource Usage (docker stats)"
echo -e "${BLUE}Capturing resource snapshot...${NC}"
ssh "$SERVER" "docker stats --no-stream $CONTAINER_NAME --format 'CPU: {{.CPUPerc}}\nMemory: {{.MemUsage}} ({{.MemPerc}})\nNetwork I/O: {{.NetIO}}\nBlock I/O: {{.BlockIO}}\nPIDs: {{.PIDs}}'" | tee -a "$OUTPUT_FILE"

# 3. File Descriptors
section "File Descriptor Usage"
echo -e "${BLUE}Checking file descriptors...${NC}"
FD_COUNT=$(ssh "$SERVER" "docker exec $CONTAINER_NAME bash -c 'ls -la /proc/1/fd 2>/dev/null | wc -l'" 2>/dev/null || echo "0")
FD_LIMIT=$(ssh "$SERVER" "docker exec $CONTAINER_NAME bash -c 'ulimit -n 2>/dev/null'" 2>/dev/null || echo "unknown")
echo "Open file descriptors: $FD_COUNT" | tee -a "$OUTPUT_FILE"
echo "FD Limit: $FD_LIMIT" | tee -a "$OUTPUT_FILE"
if [ "$FD_LIMIT" != "unknown" ] && [ "$FD_COUNT" != "0" ]; then
    FD_PERCENT=$((FD_COUNT * 100 / FD_LIMIT))
    echo "FD Usage: ${FD_PERCENT}%" | tee -a "$OUTPUT_FILE"
    if [ $FD_PERCENT -gt 90 ]; then
        echo -e "${RED}❌ CRITICAL: FD usage at ${FD_PERCENT}%${NC}" | tee -a "$OUTPUT_FILE"
    elif [ $FD_PERCENT -gt 70 ]; then
        echo -e "${YELLOW}⚠  WARNING: FD usage at ${FD_PERCENT}%${NC}" | tee -a "$OUTPUT_FILE"
    fi
fi

# 4. Network Connection States
section "Network Connection States"
echo -e "${BLUE}Analyzing network connections...${NC}"
ssh "$SERVER" "docker exec $CONTAINER_NAME netstat -an 2>/dev/null | awk '{print \$6}' | sort | uniq -c | sort -rn || echo 'netstat not available'" | tee -a "$OUTPUT_FILE"

# 5. WebSocket Connections
section "WebSocket Connections (Port 8080)"
echo -e "${BLUE}Counting active WebSocket connections...${NC}"
WS_COUNT=$(ssh "$SERVER" "docker exec $CONTAINER_NAME netstat -an 2>/dev/null | grep :8080 | grep ESTABLISHED | wc -l" || echo "0")
echo "Active connections: $WS_COUNT" | tee -a "$OUTPUT_FILE"

# Check for CLOSE_WAIT (FD leak indicator)
CLOSE_WAIT=$(ssh "$SERVER" "docker exec $CONTAINER_NAME netstat -an 2>/dev/null | grep CLOSE_WAIT | wc -l" || echo "0")
echo "CLOSE_WAIT connections: $CLOSE_WAIT" | tee -a "$OUTPUT_FILE"
if [ "$CLOSE_WAIT" -gt 100 ]; then
    echo -e "${RED}❌ CRITICAL: ${CLOSE_WAIT} connections in CLOSE_WAIT (possible FD leak)${NC}" | tee -a "$OUTPUT_FILE"
elif [ "$CLOSE_WAIT" -gt 50 ]; then
    echo -e "${YELLOW}⚠  WARNING: ${CLOSE_WAIT} connections in CLOSE_WAIT${NC}" | tee -a "$OUTPUT_FILE"
fi

# 6. Socket Summary
section "Socket Summary"
echo -e "${BLUE}Getting socket statistics...${NC}"
ssh "$SERVER" "docker exec $CONTAINER_NAME ss -s 2>/dev/null || echo 'ss not available'" | tee -a "$OUTPUT_FILE"

# 7. TCP Listen Queue
section "TCP Listen Queue (Backlog)"
echo -e "${BLUE}Checking TCP backlog...${NC}"
ssh "$SERVER" "docker exec $CONTAINER_NAME ss -ltn 2>/dev/null | grep :8080 || echo 'No listening sockets found'" | tee -a "$OUTPUT_FILE"

# 8. Thread Count
section "Thread Count"
echo -e "${BLUE}Counting threads...${NC}"
THREAD_COUNT=$(ssh "$SERVER" "docker exec $CONTAINER_NAME bash -c 'ps -eLf 2>/dev/null | wc -l'" || echo "unknown")
echo "Total threads: $THREAD_COUNT" | tee -a "$OUTPUT_FILE"
if [ "$THREAD_COUNT" != "unknown" ]; then
    if [ "$THREAD_COUNT" -gt 500 ]; then
        echo -e "${RED}❌ CRITICAL: ${THREAD_COUNT} threads (possible thread leak)${NC}" | tee -a "$OUTPUT_FILE"
    elif [ "$THREAD_COUNT" -gt 100 ]; then
        echo -e "${YELLOW}⚠  WARNING: ${THREAD_COUNT} threads${NC}" | tee -a "$OUTPUT_FILE"
    fi
fi

# 9. Memory Info
section "Memory Usage"
echo -e "${BLUE}Checking memory...${NC}"
ssh "$SERVER" "docker exec $CONTAINER_NAME cat /proc/meminfo 2>/dev/null | grep -E 'MemTotal|MemFree|MemAvailable' || echo 'meminfo not available'" | tee -a "$OUTPUT_FILE"

# 10. Application Health Check
section "Health Check"
echo -e "${BLUE}Testing health endpoint...${NC}"
HEALTH_RESPONSE=$(ssh "$SERVER" "timeout 5 curl -s http://localhost:8080/health 2>&1" || echo "TIMEOUT")
echo "Response: $HEALTH_RESPONSE" | tee -a "$OUTPUT_FILE"
if [ "$HEALTH_RESPONSE" != "OK" ]; then
    echo -e "${RED}❌ CRITICAL: Health check failed or timed out${NC}" | tee -a "$OUTPUT_FILE"
else
    echo -e "${GREEN}✓ Health check OK${NC}" | tee -a "$OUTPUT_FILE"
fi

# 11. Prometheus Metrics
section "Prometheus Metrics (Key Metrics)"
echo -e "${BLUE}Fetching application metrics...${NC}"
ssh "$SERVER" "timeout 5 curl -s http://localhost:8080/metrics 2>&1 | grep -E 'connection|websocket|request|error|group' | head -20 || echo 'Metrics unavailable'" | tee -a "$OUTPUT_FILE"

# 12. Recent Application Logs
section "Recent Application Logs (last 100 lines)"
echo -e "${BLUE}Fetching recent logs...${NC}"
ssh "$SERVER" "docker logs --tail 100 --timestamps $CONTAINER_NAME 2>&1" | tee -a "$OUTPUT_FILE"

# 13. Error Summary from Logs
section "Error Summary (last 200 lines)"
echo -e "${BLUE}Analyzing errors in logs...${NC}"
ERROR_COUNT=$(ssh "$SERVER" "docker logs --tail 200 $CONTAINER_NAME 2>&1 | grep -i 'error\|panic\|fatal' | wc -l" || echo "0")
echo "Errors found: $ERROR_COUNT" | tee -a "$OUTPUT_FILE"
if [ "$ERROR_COUNT" -gt 10 ]; then
    echo -e "${YELLOW}⚠  WARNING: $ERROR_COUNT errors in recent logs${NC}" | tee -a "$OUTPUT_FILE"
    echo "Sample errors:" | tee -a "$OUTPUT_FILE"
    ssh "$SERVER" "docker logs --tail 200 $CONTAINER_NAME 2>&1 | grep -i 'error\|panic\|fatal' | head -10" | tee -a "$OUTPUT_FILE"
fi

# 14. Tokio Console Snapshot
section "Tokio Console Async Runtime State"
echo -e "${BLUE}Capturing tokio-console snapshot (multiple views)...${NC}"
SESSION="comprehensive_diag_$$"

# Capture default view (sorted by Total time)
echo "View 1: Default (sorted by Total time)" | tee -a "$OUTPUT_FILE"
TOKIO_OUTPUT=$(ssh "$SERVER" bash <<EOF
source ~/.cargo/env 2>/dev/null || true
TERM=xterm-256color tmux new-session -d -s "$SESSION" -x 250 -y 60 "tokio-console http://localhost:6669" 2>/dev/null
sleep 6
tmux capture-pane -t "$SESSION" -p -S - 2>/dev/null
EOF
)

if [ -n "$TOKIO_OUTPUT" ]; then
    echo "$TOKIO_OUTPUT" | tee -a "$OUTPUT_FILE"
else
    echo "tokio-console not available or failed to capture" | tee -a "$OUTPUT_FILE"
fi

# Capture sorted by Busy time (blocking tasks appear first)
echo "" | tee -a "$OUTPUT_FILE"
echo "View 2: Sorted by Busy time (blocking/spinning tasks first)" | tee -a "$OUTPUT_FILE"
TOKIO_BUSY=$(ssh "$SERVER" bash <<EOF
source ~/.cargo/env 2>/dev/null || true
if tmux has-session -t "$SESSION" 2>/dev/null; then
    # Navigate to Busy column and sort descending
    tmux send-keys -t "$SESSION" 'h' 'h' 'h' 'h' 'h' 2>/dev/null  # Move to Busy column
    tmux send-keys -t "$SESSION" 'i' 2>/dev/null                   # Invert (highest first)
    sleep 2
    tmux capture-pane -t "$SESSION" -p -S - 2>/dev/null
    tmux kill-session -t "$SESSION" 2>/dev/null
fi
EOF
)

if [ -n "$TOKIO_BUSY" ]; then
    echo "$TOKIO_BUSY" | tee -a "$OUTPUT_FILE"
else
    echo "Could not capture busy-sorted view" | tee -a "$OUTPUT_FILE"
fi

# 15. Console Dump (Detailed Task Info via gRPC)
section "Console Dump - Detailed Task Information"
echo -e "${BLUE}Attempting to get detailed task info via console_dump...${NC}"

# Check if console_dump binary exists in container
if ssh "$SERVER" "docker exec $CONTAINER_NAME test -f ./console_dump" 2>/dev/null; then
    echo -e "${GREEN}✓ console_dump binary found${NC}" | tee -a "$OUTPUT_FILE"

    # Run console_dump with blocking-only and min-busy-time filters
    ssh "$SERVER" "docker exec $CONTAINER_NAME timeout 10 ./console_dump --blocking-only --min-busy-ms 50 2>&1" | tee -a "$OUTPUT_FILE"
else
    echo -e "${YELLOW}console_dump binary not available (requires rebuild with console-dump feature)${NC}" | tee -a "$OUTPUT_FILE"
    echo "To enable: Rebuild Docker image with updated Dockerfile" | tee -a "$OUTPUT_FILE"
fi

# Add note about manual detailed task inspection
{
    echo ""
    echo "For interactive detailed task inspection:"
    echo "  ssh $SERVER"
    echo "  source ~/.cargo/env"
    echo "  tokio-console http://localhost:6669"
    echo "  Navigate to a task (↑↓ or j,k) and press Enter to view full details"
} | tee -a "$OUTPUT_FILE"

# Final summary
section "Auto-Analysis Summary"
echo "" | tee -a "$OUTPUT_FILE"
echo "Diagnostic snapshot completed at: $(date)" | tee -a "$OUTPUT_FILE"
echo "" | tee -a "$OUTPUT_FILE"

# Collect all warnings/criticals
ISSUES=0

if [ "$FD_LIMIT" != "unknown" ] && [ "$FD_COUNT" != "0" ]; then
    FD_PERCENT=$((FD_COUNT * 100 / FD_LIMIT))
    if [ $FD_PERCENT -gt 90 ]; then
        echo -e "${RED}❌ CRITICAL: File descriptors at ${FD_PERCENT}%${NC}"
        ((ISSUES++))
    elif [ $FD_PERCENT -gt 70 ]; then
        echo -e "${YELLOW}⚠  WARNING: File descriptors at ${FD_PERCENT}%${NC}"
        ((ISSUES++))
    fi
fi

if [ "$CLOSE_WAIT" -gt 100 ]; then
    echo -e "${RED}❌ CRITICAL: ${CLOSE_WAIT} CLOSE_WAIT connections (FD leak)${NC}"
    ((ISSUES++))
elif [ "$CLOSE_WAIT" -gt 50 ]; then
    echo -e "${YELLOW}⚠  WARNING: ${CLOSE_WAIT} CLOSE_WAIT connections${NC}"
    ((ISSUES++))
fi

if [ "$ERROR_COUNT" -gt 10 ]; then
    echo -e "${YELLOW}⚠  WARNING: ${ERROR_COUNT} errors in recent logs${NC}"
    ((ISSUES++))
fi

if [ "$HEALTH_RESPONSE" != "OK" ]; then
    echo -e "${RED}❌ CRITICAL: Health check failed${NC}"
    ((ISSUES++))
fi

if echo "$TOKIO_OUTPUT" | grep -q "tasks have lost their wakers"; then
    LOST_WAKERS=$(echo "$TOKIO_OUTPUT" | grep -o "[0-9]* tasks have lost their wakers" | head -1)
    echo -e "${YELLOW}⚠  $LOST_WAKERS${NC}"
    ((ISSUES++))
fi

# Check for blocking tasks with high busy time
if echo "$TOKIO_BUSY" | grep -q "blocking.*[0-9]*m[0-9]*s.*[0-9]*m[0-9]*s"; then
    BLOCKING_COUNT=$(echo "$TOKIO_BUSY" | grep "blocking" | grep -E "[0-9]+m[0-9]+s.*[0-9]+m[0-9]+s" | wc -l)
    if [ "$BLOCKING_COUNT" -gt 0 ]; then
        echo -e "${YELLOW}⚠  Found $BLOCKING_COUNT blocking task(s) with high busy time${NC}"
        echo -e "${YELLOW}   (See 'View 2' section for details)${NC}"
        ((ISSUES++))
    fi
fi

echo ""
if [ $ISSUES -eq 0 ]; then
    echo -e "${GREEN}✓ No critical issues detected${NC}"
else
    echo -e "${YELLOW}Found $ISSUES potential issue(s) - review the full report${NC}"
fi

echo ""
echo "Full diagnostic report saved to:"
echo "  $OUTPUT_FILE"
echo ""
echo "To view:"
echo "  less -R $OUTPUT_FILE"
echo ""
