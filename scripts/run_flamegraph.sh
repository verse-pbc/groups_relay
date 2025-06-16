#!/bin/bash

# Script to run flamegraph on the groups_relay server

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
# Assume the project root is one level up from the scripts directory
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT" || exit 1

echo "Ensuring groups_relay release build is up-to-date..."
cargo build --release --bin groups_relay

if [ $? -ne 0 ]; then
  echo "Error: Release build failed. Aborting."
  exit 1
fi

echo ""
echo "Starting flamegraph for groups_relay..."
echo "The server will start under the profiler."
echo "Its configuration will be loaded from: $PROJECT_ROOT/config"
echo ""
echo "IMPORTANT:"
echo "1. Wait for the server to indicate it's listening (e.g., on 0.0.0.0:8080)."
echo "2. In a SEPARATE terminal, apply a workload to the server (e.g., using oha, wrk, curl)."
echo "3. After sufficient workload duration, press Ctrl+C in THIS terminal to stop profiling."
echo "   This will generate 'flamegraph.svg' in the project root: $PROJECT_ROOT"
echo ""

# Run flamegraph, passing the config directory to the binary
# sudo is required for dtrace on macOS
sudo cargo flamegraph --release --bin groups_relay -- --config-dir config

if [ $? -eq 0 ]; then
  echo ""
  echo "Flamegraph generation successful!"
  echo "Output file: $PROJECT_ROOT/flamegraph.svg"
else
  echo ""
  echo "Error: Flamegraph generation failed."
  exit 1
fi

exit 0