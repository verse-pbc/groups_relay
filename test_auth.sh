#!/bin/bash

# Start server in background
echo "Starting server..."
cargo run -- --config-dir config > test_auth_server.log 2>&1 &
SERVER_PID=$!

# Wait for server to start
echo "Waiting for server to start..."
sleep 10

echo -e "\n=== Test 1: Simple connection without auth ==="
nak req -k 1 -l 1 'ws://localhost:8080' 2>&1

echo -e "\n=== Test 2: Connection with auth flag ==="
nak req -k 1 -l 1 --auth --sec='6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e' 'ws://localhost:8080' 2>&1

echo -e "\n=== Test 3: Try to access a private group (should require auth) ==="
# First create a private group
GROUP_ID=$(date +%s)
echo "Creating private group with ID: $GROUP_ID"
nak event -k 9007 -t h="$GROUP_ID" -t private -t closed --sec='6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e' 'ws://localhost:8080'

# Wait for group to be created
sleep 2

# Try to query without auth (should fail)
echo -e "\nQuerying private group without auth:"
nak req -k 39000 -t d="$GROUP_ID" 'ws://localhost:8080' 2>&1

# Try to query with auth (should work)
echo -e "\nQuerying private group with auth:"
nak req -k 39000 -t d="$GROUP_ID" --auth --sec='6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e' 'ws://localhost:8080' 2>&1

echo -e "\n=== Checking server logs for AUTH messages ==="
grep -i "auth\|challenge\|22242" test_auth_server.log | tail -20

# Cleanup
echo -e "\nCleaning up..."
kill $SERVER_PID 2>/dev/null