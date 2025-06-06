# List available commands
default:
    @just --list

# Run all tests
test: test-unit test-integration

# Run NIP-29 comprehensive test (interactive mode)
test-nip29:
    @./scripts/flow29_test.sh manual

# Run NIP-29 comprehensive test (automated mode)
test-nip29-auto:
    @./scripts/flow29_test.sh auto

# Run NIP-29 test and analyze with Claude slash command
# Usage: just test-nip29-analyze | claude /project:analyze-nip29
test-nip29-analyze:
    @./scripts/run_nip29_test.sh

# Run unit tests with nextest
test-unit:
    cargo nextest run --lib --all-features

# Run integration tests with nextest
test-integration:
    cargo nextest run --test '*' --all-features

# Run tests with coverage
coverage:
    cargo tarpaulin --workspace --exclude-files "*/bin/*" --exclude-files "*/examples/*" --exclude-files "*/tests/*" --exclude-files "*/benches/*" --out Html --out Xml --output-dir coverage

# Clean build artifacts and logs
clean:
    cargo clean
    rm -f test_*.log
    rm -f server.log flow29_output.log

# Run the relay server
run:
    cargo run --bin groups_relay -- --config-dir crates/groups_relay/config

# Run the relay server with debug logging
run-debug:
    RUST_LOG=debug cargo run --bin groups_relay -- --config-dir crates/groups_relay/config

# Format code
fmt:
    cargo fmt --all

# Check code formatting
fmt-check:
    cargo fmt --all -- --check

# Run clippy
clippy:
    cargo clippy --workspace --all-features -- -D warnings

# Run all checks (format, clippy, tests)
check: fmt-check clippy test

# Build release version
build-release:
    cargo build --release --workspace

# Run benchmarks
bench:
    cargo bench --workspace

# Update dependencies
update:
    cargo update

# Install development tools
install-tools:
    cargo install cargo-nextest
    cargo install cargo-tarpaulin
    cargo install just