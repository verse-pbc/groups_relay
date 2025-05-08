#!/bin/bash

# Exit immediately if a command exits with a non-zero status
set -e

# ----------------------------
# Configuration and Variables
# ----------------------------

# Relay Configuration
RELAY_URL="ws://0.0.0.0:8080"
RELAY_PRIVATE_KEY="6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e"
USER_PRIVATE_KEY="262f9c9cdd4d490f54c7333c0ae7033b03cfb8f83c123f2da4e3cf10b7d33b00"
NEW_USER_PRIVATE_KEY="efa1aa99103d56f1c0d77b6986d06d4a8327c88886ed5ec0a2ed2b1bca504895"
USER_PUBLIC_KEY="e44bb8c424c2e9b6a74620ca038ad93cce3a11d6a1b4f4ae17211bb78013d972"
NEW_USER_PUBLIC_KEY="1b45eccc033451d763a71cb8ddd39dcf31b7d2d72d281c70736c8f38b2c55762"
GROUP_ID="$(date +%s)"

# Add a third user's keys
THIRD_USER_PRIVATE_KEY="7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9b"
THIRD_USER_PUBLIC_KEY="0af9b7a02a3ce9ecff15da83adeb6b0748eb2c7e325ffc7fe180b547afa0017f"

# ----------------------------
# Helper Functions
# ----------------------------

check_nak_installed() {
    if ! command -v nak &> /dev/null
    then
        echo "Error: nak is not installed. Please install nak using 'cargo install nak'."
        exit 1
    fi
}

run_step() {
    local step_number=$1
    local description=$2
    local command=$3

    echo -e "\n=== Step ${step_number}: ${description} ==="
    echo "Command to run:"
    echo -e "\033[36m${command}\033[0m"  # Cyan color for command
    read -p "Press Enter to execute this step..."

    eval "${command}"
    echo "Step ${step_number} completed."
}

# ----------------------------
# Main Execution Flow
# ----------------------------

main() {
    check_nak_installed

    echo "=== NIP-29 Group Lifecycle Test ==="

    echo -e "\n=== Group Creation Flow ==="
    run_step 1 "Admin creates group (9007)" \
        "nak event -k 9007 -t h='${GROUP_ID}' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"
    echo "Relay automatically creates 39000, 39001, 39002, and 39003 events"

    echo -e "\n=== Edit Metadata Flow ==="
    run_step 2 "Admin edits metadata (9002)" \
        "nak event -k 9002 -t h='${GROUP_ID}' -t name='Pizza Lovers' -t about='A group for pizza enthusiasts' -t picture='https://example.com/pizza.jpg' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"
    echo "Relay automatically updates 39000 metadata"

    echo -e "\n=== User Joining With Invite Code Flow ==="
    run_step 3 "Admin creates invite (9009)" \
        "nak event -k 9009 -t h='${GROUP_ID}' -t code='PIZZA123' -t roles='member' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"

    run_step 4 "First user joins with invite code (9021)" \
        "nak event -k 9021 -t h='${GROUP_ID}' -t code='PIZZA123' --auth --sec='${NEW_USER_PRIVATE_KEY}' '${RELAY_URL}'"
    echo "Relay automatically creates 9000 and updates 39002"

    run_step 5 "First user posts message to group (1)" \
        "nak event -k 9 -c 'Hello, fellow pizza lovers!' -t h='${GROUP_ID}' --auth --sec='${NEW_USER_PRIVATE_KEY}' '${RELAY_URL}'"

    echo -e "\n=== Manual Join Request Flow ==="
    run_step 6 "Second user requests to join without code (9021)" \
        "nak event -k 9021 -t h='${GROUP_ID}' --auth --sec='${THIRD_USER_PRIVATE_KEY}' '${RELAY_URL}'"

    run_step 7 "Admin manually adds second user (9000)" \
        "nak event -k 9000 -t h='${GROUP_ID}' -t 'p=${THIRD_USER_PUBLIC_KEY};role=member' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"
    echo "Relay automatically updates 39002"

    echo -e "\n=== First user lists all events for -h tag without auth ==="
    run_step 9 "First user lists all events for -h tag (9000)" \
        "nak req -t h='${GROUP_ID}' '${RELAY_URL}'"

    echo -e "\n=== First user lists all events for -h tag with auth ==="
    run_step 10 "First user lists all events for -h tag (9000)" \
        "nak req -t h='${GROUP_ID}' -fpa --auth --sec='${NEW_USER_PRIVATE_KEY}' '${RELAY_URL}'"

    echo -e "\n=== User Leaving Flow ==="
    run_step 11 "First user requests to leave (9022)" \
        "nak event -k 9022 -t h='${GROUP_ID}' --auth --sec='${NEW_USER_PRIVATE_KEY}' '${RELAY_URL}'"
    echo "Relay automatically creates 9001 and updates 39002"

    echo -e "\n=== First user lists all events for -h tag ==="
    run_step 12 "First user lists all events for -h tag (9000)" \
        "nak req -t h='${GROUP_ID}' -fpa --auth --sec='${NEW_USER_PRIVATE_KEY}' '${RELAY_URL}'"

    echo -e "\n=== Moderation Actions Flow ==="
    run_step 13 "Admin removes second user (9001)" \
        "nak event -k 9001 -t h='${GROUP_ID}' -t p='${THIRD_USER_PUBLIC_KEY}' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"
    echo "Relay automatically updates 39002"

    echo -e "\n=== Delete Message ==="
    MESSAGE_ID=`nak req -k 9 -l 1 -t h=${GROUP_ID} -fpa --auth --sec=${RELAY_PRIVATE_KEY} ${RELAY_URL} |jq -r '.id'`
    run_step 14 "Admin deletes message (9005)" \
        "nak event -k 9005 -t h='${GROUP_ID}' -t e='${MESSAGE_ID}' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"

    echo -e "\n=== Delete Group ==="
    run_step 15 "Admin deletes group (9006)" \
        "nak event -k 9008 -t h='${GROUP_ID}' --auth --sec='${RELAY_PRIVATE_KEY}' '${RELAY_URL}'"
}

main
