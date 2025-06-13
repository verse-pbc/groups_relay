#!/bin/bash

# Commit 1: Remove profile_aggregator from workspace
git add Cargo.toml
git add Cargo.lock
git commit -m "Remove profile_aggregator from workspace members"

# Commit 2: Remove relay_consumer_server crate
git add crates/relay_consumer_server/
git commit -m "Remove deprecated relay_consumer_server crate"

# Commit 3: Add profile_aggregator as untracked (to be moved to separate repo)
# Skip adding profile_aggregator since it will be moved to a separate repository
echo "Note: crates/profile_aggregator/ should be removed or moved to a separate repository"
echo "Run: rm -rf crates/profile_aggregator/"