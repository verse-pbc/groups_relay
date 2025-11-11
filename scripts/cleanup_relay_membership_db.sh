#!/bin/bash
# Delete incorrect kind:9000 events from database using the delete_event binary
# These events were created before the NIP-29 fix where relay added itself as member
#
# Usage: ./scripts/cleanup_relay_membership_db.sh communities

set -e

SERVER="${1:-communities}"
RELAY_PUBKEY="65e2b56139cbd99aaa5d114898882a6e9be53eae18987c70ae1598d1a157c8f6"

echo "=========================================="
echo "Database Cleanup - Remove Bad kind:9000 Events"
echo "=========================================="
echo "Server: $SERVER"
echo ""

# Get event IDs locally first
echo "Step 1: Fetching list of bad kind:9000 events..."

if [ "$SERVER" = "communities" ]; then
    RELAY_URL="wss://communities.nos.social"
    RELAY_SEC="$COMMUNITIES"
elif [ "$SERVER" = "communities2" ]; then
    RELAY_URL="wss://communities2.nos.social"
    RELAY_SEC="$COMMUNITIES2"
else
    echo "Error: Unknown server"
    exit 1
fi

# Get all bad event IDs
BAD_EVENT_IDS=$(nak req -k 9000 --tag p="$RELAY_PUBKEY" --auth --fpa --sec="$RELAY_SEC" "$RELAY_URL" 2>&1 | grep '"kind":9000' | grep -o '"id":"[^"]*"' | cut -d'"' -f4)

if [ -z "$BAD_EVENT_IDS" ]; then
    echo "✅ No bad events found!"
    exit 0
fi

NUM_EVENTS=$(echo "$BAD_EVENT_IDS" | wc -l | tr -d ' ')
echo "Found $NUM_EVENTS bad kind:9000 events"
echo ""
echo "Preview (first 5):"
echo "$BAD_EVENT_IDS" | head -5
echo "..."
echo ""

read -p "Delete these $NUM_EVENTS events from database? (yes/no): " CONFIRM
if [ "$CONFIRM" != "yes" ]; then
    echo "Aborted."
    exit 0
fi

echo ""
echo "Step 2: Deleting events from database..."
echo ""

# Create temp file with event IDs on server
TEMP_FILE="/tmp/bad_events_$(date +%s).txt"
echo "$BAD_EVENT_IDS" | ssh "$SERVER" "cat > $TEMP_FILE"

# SSH to server and delete each event
DELETED=0
FAILED=0

while IFS= read -r event_id; do
    echo -n "Deleting event $event_id... "
    
    if ssh "$SERVER" "docker exec groups_relay /app/delete_event --db /data --event-id $event_id --yes" 2>&1 | grep -q "Successfully deleted"; then
        echo "✓"
        ((DELETED++))
    else
        echo "✗ (may not exist or already deleted)"
        ((FAILED++))
    fi
done <<< "$BAD_EVENT_IDS"

echo ""
echo "=========================================="
echo "Summary"
echo "=========================================="
echo "Total bad events: $NUM_EVENTS"
echo "Successfully deleted: $DELETED"
echo "Failed/Not found: $FAILED"
echo ""
echo "⚠️  IMPORTANT: Restart the relay for changes to take effect:"
echo "   ssh $SERVER 'cd ~/groups_relay && docker-compose restart groups_relay'"
echo ""
