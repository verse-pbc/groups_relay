#!/bin/bash

# Comprehensive profiling script for groups_relay
# Combines flamegraph generation, load testing, and analysis in one tool

set -euo pipefail

# Get script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Default values
WARMUP_TIME=10
LOAD_CONNECTIONS=1000
PROFILE_DURATION=30
OUTPUT_NAME="profile"
RUST_LOG_LEVEL="warn"
GENERATE_LOAD=true
ANALYZE_ONLY=""

# Help function
show_help() {
    cat << EOF
${BLUE}Groups Relay Profiling Tool${NC}

${YELLOW}Usage:${NC}
    $0 [options]

${YELLOW}Options:${NC}
    -w, --warmup SECONDS      Warmup time after server starts (default: 10)
    -n, --connections NUM     Number of test connections (default: 1000)
    -d, --duration SECONDS    Profiling duration (default: 30)
    -o, --output NAME         Output filename prefix (default: profile)
    -l, --log-level LEVEL     RUST_LOG level (default: warn)
    --no-load                 Skip load generation (useful for manual testing)
    --analyze FILE            Only analyze existing flamegraph SVG file
    -h, --help                Show this help message

${YELLOW}Examples:${NC}
    # Quick profile with defaults
    $0

    # Production-like profile
    $0 -w 15 -n 2000 -d 60 -o production_profile -l warn

    # Profile without load generation (manual testing)
    $0 --no-load -o manual_test

    # Analyze existing flamegraph
    $0 --analyze flamegraph.svg

${YELLOW}Output:${NC}
    Creates the following files:
    - {output}_flamegraph.svg     Visual flamegraph
    - {output}_analysis.txt       Detailed analysis
    - {output}_summary.txt        Quick summary

${YELLOW}Requirements:${NC}
    - cargo-flamegraph (cargo install flamegraph)
    - Python 3 with websockets (pip install websockets)
    - sudo access for system profiling

EOF
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -w|--warmup)
            WARMUP_TIME="$2"
            shift 2
            ;;
        -n|--connections)
            LOAD_CONNECTIONS="$2"
            shift 2
            ;;
        -d|--duration)
            PROFILE_DURATION="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_NAME="$2"
            shift 2
            ;;
        -l|--log-level)
            RUST_LOG_LEVEL="$2"
            shift 2
            ;;
        --no-load)
            GENERATE_LOAD=false
            shift
            ;;
        --analyze)
            ANALYZE_ONLY="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            show_help
            exit 1
            ;;
    esac
done

# Function to analyze flamegraph
analyze_flamegraph() {
    local svg_file="$1"
    local output_prefix="$2"
    
    echo -e "${BLUE}Analyzing flamegraph...${NC}"
    
    # Create Python analyzer inline
    cat > /tmp/analyze_flamegraph.py << 'PYTHON_SCRIPT'
#!/usr/bin/env python3
import re
import sys
from collections import defaultdict

def parse_flamegraph_svg(svg_file):
    functions = defaultdict(int)
    total_samples = 0
    
    with open(svg_file, 'r') as f:
        content = f.read()
    
    total_match = re.search(r'total_samples="(\d+)"', content)
    if total_match:
        total_samples = int(total_match.group(1))
    
    pattern = r'<title>([^<]+)\s+\((\d+)\s+samples?,\s+[\d.]+%\)</title>'
    for match in re.finditer(pattern, content):
        function_name = match.group(1).strip()
        samples = int(match.group(2))
        functions[function_name] += samples
    
    return functions, total_samples

def analyze(svg_file, output_prefix):
    functions, total_samples = parse_flamegraph_svg(svg_file)
    
    if total_samples == 0:
        print("Error: No samples found in flamegraph")
        return
    
    # Sort by sample count
    sorted_functions = sorted(functions.items(), key=lambda x: x[1], reverse=True)
    
    # Write detailed analysis
    with open(f"{output_prefix}_analysis.txt", 'w') as f:
        f.write(f"Flamegraph Analysis\n")
        f.write(f"==================\n\n")
        f.write(f"Total samples: {total_samples:,}\n\n")
        
        f.write("Top 50 Functions by CPU Time:\n")
        f.write("-" * 80 + "\n")
        f.write(f"{'%CPU':<8} {'Samples':<10} {'Function'}\n")
        f.write("-" * 80 + "\n")
        
        for func, samples in sorted_functions[:50]:
            percentage = (samples / total_samples) * 100
            f.write(f"{percentage:6.2f}%  {samples:<10,} {func}\n")
    
    # Write summary
    with open(f"{output_prefix}_summary.txt", 'w') as f:
        f.write("Performance Profile Summary\n")
        f.write("==========================\n\n")
        
        # Categorize functions
        categories = {
            'Middleware': 0,
            'Database': 0,
            'WebSocket': 0,
            'Tokio Runtime': 0,
            'System': 0,
            'Logging': 0,
        }
        
        for func, samples in functions.items():
            percentage = (samples / total_samples) * 100
            
            if 'middleware' in func.lower():
                categories['Middleware'] += percentage
            elif any(x in func.lower() for x in ['heed', 'lmdb', 'database', 'store']):
                categories['Database'] += percentage
            elif any(x in func.lower() for x in ['websocket', 'tungstenite', 'ws']):
                categories['WebSocket'] += percentage
            elif 'tokio' in func.lower():
                categories['Tokio Runtime'] += percentage
            elif any(x in func.lower() for x in ['pthread', 'clock', 'mach_', '__']):
                categories['System'] += percentage
            elif any(x in func.lower() for x in ['log', 'trace', 'debug']):
                categories['Logging'] += percentage
        
        f.write("CPU Usage by Category:\n")
        for category, percent in sorted(categories.items(), key=lambda x: x[1], reverse=True):
            if percent > 0:
                f.write(f"  {category:<20} {percent:6.2f}%\n")
        
        f.write(f"\nTop 10 Hotspots:\n")
        for i, (func, samples) in enumerate(sorted_functions[:10], 1):
            percentage = (samples / total_samples) * 100
            f.write(f"  {i:2d}. {percentage:5.2f}% - {func[:60]}{'...' if len(func) > 60 else ''}\n")
    
    print(f"Analysis saved to:")
    print(f"  - {output_prefix}_analysis.txt")
    print(f"  - {output_prefix}_summary.txt")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Usage: analyze_flamegraph.py <svg_file> <output_prefix>")
        sys.exit(1)
    
    analyze(sys.argv[1], sys.argv[2])
PYTHON_SCRIPT
    
    python3 /tmp/analyze_flamegraph.py "$svg_file" "$output_prefix"
    rm -f /tmp/analyze_flamegraph.py
}

# If only analyzing, do that and exit
if [ -n "$ANALYZE_ONLY" ]; then
    if [ ! -f "$ANALYZE_ONLY" ]; then
        echo -e "${RED}Error: File not found: $ANALYZE_ONLY${NC}"
        exit 1
    fi
    
    output_base="${ANALYZE_ONLY%.svg}"
    analyze_flamegraph "$ANALYZE_ONLY" "$output_base"
    
    # Show summary
    if [ -f "${output_base}_summary.txt" ]; then
        echo ""
        cat "${output_base}_summary.txt"
    fi
    exit 0
fi

# Main profiling workflow
cd "$PROJECT_ROOT"

echo -e "${BLUE}Groups Relay Performance Profiling${NC}"
echo -e "${YELLOW}=================================${NC}"
echo "Configuration:"
echo "  Warmup time: ${WARMUP_TIME}s"
echo "  Test connections: $LOAD_CONNECTIONS"
echo "  Profile duration: ${PROFILE_DURATION}s"
echo "  Output prefix: $OUTPUT_NAME"
echo "  Log level: $RUST_LOG_LEVEL"
echo "  Generate load: $GENERATE_LOAD"
echo ""

# Clean up
echo -e "${BLUE}Cleaning up...${NC}"
# Force remove old traces (may require sudo)
if [ -d "cargo-flamegraph.trace" ]; then
    echo "Removing old trace directory..."
    sudo rm -rf cargo-flamegraph.trace 2>/dev/null || rm -rf cargo-flamegraph.trace 2>/dev/null || true
fi
rm -rf cargo-flamegraph.stacks 2>/dev/null || true
pkill -f groups_relay || true
sleep 1

# Build
echo -e "${BLUE}Building release version with debug symbols...${NC}"
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --bin groups_relay

if [ $? -ne 0 ]; then
    echo -e "${RED}Error: Build failed${NC}"
    exit 1
fi

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    
    # Find and kill relay process
    RELAY_PID=$(pgrep -f "groups_relay --config-dir" | head -1)
    if [ -n "$RELAY_PID" ]; then
        sudo kill -INT $RELAY_PID 2>/dev/null || true
    fi
    
    # Kill flamegraph process
    if [ -n "${FLAMEGRAPH_PID:-}" ]; then
        sudo kill -INT $FLAMEGRAPH_PID 2>/dev/null || true
    fi
    
    pkill -f groups_relay || true
    pkill -f test_connection_performance || true
}

trap cleanup EXIT INT TERM

# Start profiling
echo -e "${BLUE}Starting flamegraph profiling...${NC}"

if [ "$(uname)" = "Darwin" ]; then
    # macOS
    RUST_LOG="$RUST_LOG_LEVEL" CARGO_PROFILE_RELEASE_DEBUG=true sudo -E cargo flamegraph \
        --release \
        --bin groups_relay \
        --output "${OUTPUT_NAME}_flamegraph.svg" \
        -- --config-dir crates/groups_relay/config &
else
    # Linux
    RUST_LOG="$RUST_LOG_LEVEL" CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
        --release \
        --bin groups_relay \
        --output "${OUTPUT_NAME}_flamegraph.svg" \
        -- --config-dir crates/groups_relay/config &
fi

FLAMEGRAPH_PID=$!

# Wait for server to start
echo -e "${BLUE}Waiting for server to initialize...${NC}"
for i in $(seq 1 30); do
    if nc -z localhost 8080 2>/dev/null; then
        echo -e "${GREEN}Server is ready${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Error: Server failed to start${NC}"
        exit 1
    fi
    sleep 0.5
done

# Warmup period
echo -e "${BLUE}Warming up for ${WARMUP_TIME} seconds...${NC}"
for i in $(seq 1 $WARMUP_TIME); do
    printf "\r[%2d/%2d]" $i $WARMUP_TIME
    sleep 1
done
echo ""

# Generate load if requested
if [ "$GENERATE_LOAD" = true ]; then
    echo -e "${BLUE}Generating load...${NC}"
    
    # Connection test
    if command -v python3 &> /dev/null && python3 -c "import websockets" 2>/dev/null; then
        echo "Running connection performance test..."
        python3 "$SCRIPT_DIR/test_connection_performance.py" \
            "ws://localhost:8080" \
            --count "$LOAD_CONNECTIONS" \
            --mode both \
            --delay 50 &
        CONNECTION_TEST_PID=$!
    else
        echo -e "${YELLOW}Warning: Python websockets not available, using basic load generation${NC}"
    fi
    
    # Event generation
    if command -v nak &> /dev/null; then
        echo "Generating event load..."
        PRIVATE_KEY=$(nak key generate | grep -v '^pubkey:')
        END_TIME=$(($(date +%s) + PROFILE_DURATION - 5))
        
        while [ $(date +%s) -lt $END_TIME ]; do
            for i in {1..10}; do
                echo "$PRIVATE_KEY" | nak event -k 1 -c "Profile test" "ws://localhost:8080" &> /dev/null &
            done
            sleep 0.5
        done &
        EVENT_GEN_PID=$!
    fi
    
    # Wait for profiling duration
    echo "Profiling for $PROFILE_DURATION seconds..."
    for i in $(seq 1 $PROFILE_DURATION); do
        printf "\r[%3d/%3d]" $i $PROFILE_DURATION
        sleep 1
    done
    echo ""
    
    # Stop load generation
    [ -n "${CONNECTION_TEST_PID:-}" ] && kill $CONNECTION_TEST_PID 2>/dev/null || true
    [ -n "${EVENT_GEN_PID:-}" ] && kill $EVENT_GEN_PID 2>/dev/null || true
else
    echo -e "${YELLOW}Manual testing mode - generate your own load${NC}"
    echo "Profiling for $PROFILE_DURATION seconds..."
    sleep $PROFILE_DURATION
fi

# Stop profiling
echo -e "${BLUE}Stopping profiling...${NC}"
RELAY_PID=$(pgrep -f "groups_relay --config-dir" | head -1)
if [ -n "$RELAY_PID" ]; then
    sudo kill -INT $RELAY_PID 2>/dev/null || true
    sleep 2
fi

wait $FLAMEGRAPH_PID 2>/dev/null || true

# Check results
if [ -f "${OUTPUT_NAME}_flamegraph.svg" ]; then
    echo -e "${GREEN}Flamegraph generated successfully!${NC}"
    
    # Analyze
    analyze_flamegraph "${OUTPUT_NAME}_flamegraph.svg" "$OUTPUT_NAME"
    
    # Show summary
    if [ -f "${OUTPUT_NAME}_summary.txt" ]; then
        echo ""
        cat "${OUTPUT_NAME}_summary.txt"
    fi
    
    echo -e "\n${GREEN}Profiling complete!${NC}"
    echo "Files generated:"
    ls -lh ${OUTPUT_NAME}_* 2>/dev/null | grep -v ".sh"
else
    echo -e "${RED}Error: Flamegraph generation failed${NC}"
    exit 1
fi