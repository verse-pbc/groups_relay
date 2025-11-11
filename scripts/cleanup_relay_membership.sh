#!/bin/bash
# Cleanup script to delete incorrect kind:9000 events where relay added itself
# These events were created before the NIP-29 fix (commit 121d1d5)
#
# Usage: ./scripts/cleanup_relay_membership.sh communities

set -e

SERVER="${1:-communities}"
RELAY_PUBKEY="65e2b56139cbd99aaa5d114898882a6e9be53eae18987c70ae1598d1a157c8f6"

echo "=========================================="
echo "NIP-29 Bug Cleanup - Remove Relay Membership"
echo "=========================================="
echo "Server: $SERVER"
echo "Relay pubkey: $RELAY_PUBKEY"
echo ""

# Get the relay secret from environment
if [ "$SERVER" = "communities" ]; then
    if [ -z "$COMMUNITIES" ]; then
        echo "Error: COMMUNITIES environment variable not set"
        exit 1
    fi
    RELAY_SEC="$COMMUNITIES"
    RELAY_URL="wss://communities.nos.social"
elif [ "$SERVER" = "communities2" ]; then
    if [ -z "$COMMUNITIES2" ]; then
        echo "Error: COMMUNITIES2 environment variable not set"
        exit 1
    fi
    RELAY_SEC="$COMMUNITIES2"
    RELAY_URL="wss://communities2.nos.social"
else
    echo "Error: Unknown server. Use 'communities' or 'communities2'"
    exit 1
fi

echo "Step 1: Finding affected kind:9000 events..."
echo ""

# Query for all kind:9000 events where relay added itself
BAD_EVENTS=$(nak req -k 9000 --tag p="$RELAY_PUBKEY" --auth --fpa --sec="$RELAY_SEC" "$RELAY_URL" 2>&1 | grep '"kind":9000')

if [ -z "$BAD_EVENTS" ]; then
    echo "✅ No bad events found! Relay is not incorrectly listed as member in any group."
    exit 0
fi

# Count affected groups
NUM_EVENTS=$(echo "$BAD_EVENTS" | wc -l | tr -d ' ')
echo "Found $NUM_EVENTS bad kind:9000 events where relay added itself"
echo ""

# Extract event IDs
EVENT_IDS=$(echo "$BAD_EVENTS" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)

echo "Step 2: Preview of events to delete (first 5):"
echo "$EVENT_IDS" | head -5
echo "..."
echo ""

read -p "Do you want to delete these $NUM_EVENTS events? (yes/no): " CONFIRM

if [ "$CONFIRM" != "yes" ]; then
    echo "Aborted."
    exit 0
fi

echo ""
echo "Step 3: Creating kind:5 deletion request..."
echo ""

# Build the -e tag arguments for nak
E_TAGS=""
for event_id in $EVENT_IDS; do
    E_TAGS="$E_TAGS -e $event_id"
done

# Create kind:5 deletion request
# NOTE: Using multiple -e tags to reference all events at once
DELETION_CMD="nak event -k 5 -c 'Cleanup: Removing incorrect relay membership from group creation (bug fix commit 121d1d5)' $E_TAGS --auth --sec=$RELAY_SEC $RELAY_URL"

echo "Command:"
echo "$DELETION_CMD"
echo ""

read -p "Execute deletion? (yes/no): " EXECUTE

if [ "$EXECUTE" != "yes" ]; then
    echo "Aborted."
    exit 0
fi

echo ""
echo "Executing deletion request..."
eval "$DELETION_CMD"

echo ""
echo "✅ Deletion request published!"
echo ""
echo "Note: Relays will process the deletion request and stop publishing these events."
echo "The kind:5 deletion request itself will remain visible as a record."
echo ""
echo "Affected groups: $NUM_EVENTS"
echo "These groups will now have correct moderation history."
