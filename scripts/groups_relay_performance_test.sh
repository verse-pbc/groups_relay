#!/bin/bash

# Groups Relay Performance Comparison Script - Using Allowed Kinds
# Usage: ./groups_relay_performance_test.sh <relay1_url> <relay2_url> [options]
#
# Options:
#   -n <num>     Number of events to publish (default: 30)
#   -r <num>     Number of read requests (default: 30)
#   -d <ms>      Delay between requests in milliseconds (default: 200)
#   -k <key>     Private key to use (hex format, generates if not provided)
#   -v           Verbose output

set -euo pipefail

# Default values
NUM_EVENTS=30
NUM_READS=30
REQUEST_DELAY=200  # Higher default delay for groups relay
VERBOSE=false
PRIVATE_KEY=""
RELAY1_URL=""
RELAY2_URL=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m' # No Color

# Testing with Gift Wrap kind only
ALLOWED_KINDS=(1059)

# Global variables to store results
# Publishing results
RELAY1_PUB_AVG=""
RELAY1_PUB_MIN=""
RELAY1_PUB_MAX=""
RELAY1_PUB_P50=""
RELAY1_PUB_P90=""
RELAY1_PUB_P95=""
RELAY1_PUB_SUCCESS=""
RELAY1_PUB_FAILED=""
RELAY1_PUB_RATE_LIMITED=""

RELAY2_PUB_AVG=""
RELAY2_PUB_MIN=""
RELAY2_PUB_MAX=""
RELAY2_PUB_P50=""
RELAY2_PUB_P90=""
RELAY2_PUB_P95=""
RELAY2_PUB_SUCCESS=""
RELAY2_PUB_FAILED=""
RELAY2_PUB_RATE_LIMITED=""

# Reading results
RELAY1_READ_AVG=""
RELAY1_READ_MIN=""
RELAY1_READ_MAX=""
RELAY1_READ_P50=""
RELAY1_READ_P90=""
RELAY1_READ_P95=""
RELAY1_READ_SUCCESS=""
RELAY1_READ_FAILED=""
RELAY1_READ_RATE_LIMITED=""

RELAY2_READ_AVG=""
RELAY2_READ_MIN=""
RELAY2_READ_MAX=""
RELAY2_READ_P50=""
RELAY2_READ_P90=""
RELAY2_READ_P95=""
RELAY2_READ_SUCCESS=""
RELAY2_READ_FAILED=""
RELAY2_READ_RATE_LIMITED=""

# Parse command line arguments
parse_args() {
    if [ $# -lt 2 ]; then
        echo "Usage: $0 <relay1_url> <relay2_url> [options]"
        echo "Options:"
        echo "  -n <num>     Number of events to publish (default: 30)"
        echo "  -r <num>     Number of read requests (default: 30)"
        echo "  -d <ms>      Delay between requests in milliseconds (default: 200)"
        echo "  -k <key>     Private key to use (hex format)"
        echo "  -v           Verbose output"
        exit 1
    fi

    RELAY1_URL=$1
    RELAY2_URL=$2
    shift 2

    while getopts "n:r:d:k:vh" opt; do
        case $opt in
            n) NUM_EVENTS=$OPTARG ;;
            r) NUM_READS=$OPTARG ;;
            d) REQUEST_DELAY=$OPTARG ;;
            k) PRIVATE_KEY=$OPTARG ;;
            v) VERBOSE=true ;;
            h)
                echo "Usage: $0 <relay1_url> <relay2_url> [options]"
                exit 0
                ;;
            \?) echo "Invalid option -$OPTARG" >&2; exit 1 ;;
        esac
    done
}

# Check if required tools are installed
check_dependencies() {
    local missing_deps=()
    
    if ! command -v nak &> /dev/null; then
        missing_deps+=("nak")
    fi
    
    if ! command -v jq &> /dev/null; then
        missing_deps+=("jq")
    fi
    
    if ! command -v bc &> /dev/null; then
        missing_deps+=("bc")
    fi
    
    if [ ${#missing_deps[@]} -ne 0 ]; then
        echo -e "${RED}Error: Missing required dependencies:${NC}"
        for dep in "${missing_deps[@]}"; do
            echo "  - $dep"
        done
        echo ""
        echo "Install instructions:"
        echo "  nak: go install github.com/fiatjaf/nak@latest"
        echo "  jq:  apt-get install jq (Ubuntu) or brew install jq (macOS)"
        echo "  bc:  apt-get install bc (Ubuntu) or brew install bc (macOS)"
        exit 1
    fi
}

# Generate or use provided private key
setup_keys() {
    if [ -z "$PRIVATE_KEY" ]; then
        echo -e "${BLUE}Generating temporary private key...${NC}"
        PRIVATE_KEY=$(nak key generate | grep -v '^pubkey:')
    fi
    
    PUBLIC_KEY=$(echo "$PRIVATE_KEY" | nak key public)
    
    if [ "$VERBOSE" = true ]; then
        echo -e "${GREEN}Using public key: $PUBLIC_KEY${NC}"
    fi
}

# Function to measure time in milliseconds
get_time_ms() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS - check if gdate is available
        if command -v gdate &> /dev/null; then
            echo $(($(gdate +%s%N)/1000000))
        else
            # Fallback to less precise measurement
            python3 -c 'import time; print(int(time.time() * 1000))'
        fi
    else
        # Linux
        echo $(($(date +%s%N)/1000000))
    fi
}

# Sleep for milliseconds
sleep_ms() {
    local ms=$1
    sleep $(echo "scale=3; $ms/1000" | bc)
}

# Calculate percentile
calculate_percentile() {
    local file=$1
    local percentile=$2
    local count=$(wc -l < "$file")
    local index=$(echo "scale=0; $count * $percentile / 100" | bc)
    if [ "$index" -eq 0 ]; then
        index=1
    fi
    sort -n "$file" | sed -n "${index}p"
}

# Get random allowed kind
get_random_kind() {
    echo "${ALLOWED_KINDS[$RANDOM % ${#ALLOWED_KINDS[@]}]}"
}

# Generate content based on kind
generate_content_for_kind() {
    local kind=$1
    local index=$2
    
    case $kind in
        1059)   # Gift wrap
            echo '{"wrapped": "gift-'$index'", "recipient": "test"}'
            ;;
        *)
            echo "Test event #$index for kind $kind"
            ;;
    esac
}

# Store results for later comparison
store_results() {
    local relay_name=$1
    local test_type=$2  # "publish" or "read"
    local temp_file=$3
    
    if [ ! -s "$temp_file" ]; then
        return
    fi
    
    local avg_latency=$(awk '{ sum += $1 } END { print sum/NR }' "$temp_file")
    local min_latency=$(sort -n "$temp_file" | head -1)
    local max_latency=$(sort -n "$temp_file" | tail -1)
    local p50=$(calculate_percentile "$temp_file" 50)
    local p90=$(calculate_percentile "$temp_file" 90)
    local p95=$(calculate_percentile "$temp_file" 95)
    
    if [ "$relay_name" = "relay1" ]; then
        if [ "$test_type" = "publish" ]; then
            RELAY1_PUB_AVG=$avg_latency
            RELAY1_PUB_MIN=$min_latency
            RELAY1_PUB_MAX=$max_latency
            RELAY1_PUB_P50=$p50
            RELAY1_PUB_P90=$p90
            RELAY1_PUB_P95=$p95
        else
            RELAY1_READ_AVG=$avg_latency
            RELAY1_READ_MIN=$min_latency
            RELAY1_READ_MAX=$max_latency
            RELAY1_READ_P50=$p50
            RELAY1_READ_P90=$p90
            RELAY1_READ_P95=$p95
        fi
    else
        if [ "$test_type" = "publish" ]; then
            RELAY2_PUB_AVG=$avg_latency
            RELAY2_PUB_MIN=$min_latency
            RELAY2_PUB_MAX=$max_latency
            RELAY2_PUB_P50=$p50
            RELAY2_PUB_P90=$p90
            RELAY2_PUB_P95=$p95
        else
            RELAY2_READ_AVG=$avg_latency
            RELAY2_READ_MIN=$min_latency
            RELAY2_READ_MAX=$max_latency
            RELAY2_READ_P50=$p50
            RELAY2_READ_P90=$p90
            RELAY2_READ_P95=$p95
        fi
    fi
}

# Test event publishing latency with allowed kinds
test_event_publishing() {
    local relay_url=$1
    local relay_name=$2
    local relay_color=$3
    
    echo -e "\n${relay_color}=== Testing Event Publishing Latency for $relay_name ===${NC}"
    echo "Publishing $NUM_EVENTS events to $relay_url"
    echo "Using allowed kinds: ${ALLOWED_KINDS[@]}"
    echo "Delay between requests: ${REQUEST_DELAY}ms"
    
    local successful=0
    local failed=0
    local temp_file=$(mktemp)
    local rate_limited=0
    local kinds_used=()
    
    for i in $(seq 1 $NUM_EVENTS); do
        local kind=$(get_random_kind)
        local content=$(generate_content_for_kind $kind $i)
        local start_time=$(get_time_ms)
        
        # Track which kinds we're using
        if [[ ${#kinds_used[@]} -eq 0 ]] || [[ ! " ${kinds_used[@]} " =~ " ${kind} " ]]; then
            kinds_used+=($kind)
        fi
        
        # Use timeout to prevent hanging on rate-limited requests
        if timeout 10s bash -c "echo '$PRIVATE_KEY' | nak event -k $kind -c '$content' '$relay_url' 2>&1" > /tmp/nak_output 2>&1; then
            local end_time=$(get_time_ms)
            local latency=$((end_time - start_time))
            
            # Check if we got rate limited or rejected
            if grep -q "rate\|limit\|429\|too many\|restricted\|not allowed" /tmp/nak_output 2>/dev/null; then
                ((rate_limited++))
                if [ "$VERBOSE" = true ]; then
                    echo "Event $i (kind $kind): Rate limited or rejected"
                    cat /tmp/nak_output
                fi
            else
                ((successful++))
                echo "$latency" >> "$temp_file"
                
                if [ "$VERBOSE" = true ]; then
                    echo "Event $i (kind $kind): ${latency}ms"
                fi
            fi
        else
            ((failed++))
            if [ "$VERBOSE" = true ]; then
                echo "Event $i (kind $kind): Failed or timed out"
            fi
        fi
        
        # Progress indicator
        if [ "$VERBOSE" = false ]; then
            printf "\r[%3d/%3d] Success: %3d, Failed: %3d, Rate limited/Rejected: %3d" \
                "$i" "$NUM_EVENTS" "$successful" "$failed" "$rate_limited"
        fi
        
        # Sleep between requests to avoid rate limiting
        if [ $i -lt $NUM_EVENTS ]; then
            sleep_ms "$REQUEST_DELAY"
        fi
    done
    
    echo ""
    
    # Store results
    if [ "$relay_name" = "relay1" ]; then
        RELAY1_PUB_SUCCESS=$successful
        RELAY1_PUB_FAILED=$failed
        RELAY1_PUB_RATE_LIMITED=$rate_limited
    else
        RELAY2_PUB_SUCCESS=$successful
        RELAY2_PUB_FAILED=$failed
        RELAY2_PUB_RATE_LIMITED=$rate_limited
    fi
    
    if [ -s "$temp_file" ] && [ $successful -gt 0 ]; then
        store_results "$relay_name" "publish" "$temp_file"
        
        echo -e "\n${GREEN}Publishing Results:${NC}"
        echo "  Successful: $successful / $NUM_EVENTS"
        echo "  Failed/Timeout: $failed"
        echo "  Rate limited/Rejected: $rate_limited"
    else
        echo -e "\n${RED}No successful events to calculate statistics${NC}"
    fi
    
    rm -f "$temp_file" /tmp/nak_output
}

# Test event reading latency
test_event_reading() {
    local relay_url=$1
    local relay_name=$2
    local relay_color=$3
    
    echo -e "\n${relay_color}=== Testing Event Reading Latency for $relay_name ===${NC}"
    echo "Performing $NUM_READS read requests"
    echo "Delay between requests: ${REQUEST_DELAY}ms"
    
    local successful=0
    local failed=0
    local temp_file=$(mktemp)
    local rate_limited=0
    
    # First, publish a few events to ensure there's data to read
    echo -e "${BLUE}Publishing test events for reading...${NC}"
    for i in {1..5}; do
        local kind=$(get_random_kind)
        local content=$(generate_content_for_kind $kind $i)
        echo "$PRIVATE_KEY" | nak event -k $kind -c "$content" -t "test=groups-latency" "$relay_url" &> /dev/null
        sleep_ms 200
    done
    
    sleep 1  # Give relay time to process
    
    # Test various types of queries with allowed kinds
    for i in $(seq 1 $NUM_READS); do
        local query_type=$((i % 5))
        local start_time=$(get_time_ms)
        local kind=$(get_random_kind)
        
        case $query_type in
            0)  # Query by author
                query_cmd="nak req -a '$PUBLIC_KEY' -l 200 '$relay_url'"
                ;;
            1)  # Query by specific allowed kind
                query_cmd="nak req -k $kind -l 200 '$relay_url'"
                ;;
            2)  # Query by tag
                query_cmd="nak req -t 'test=groups-latency' -l 200 '$relay_url'"
                ;;
            3)  # Query with time range
                since=$(($(date +%s) - 3600))
                query_cmd="nak req --since '$since' -k $kind -l 200 '$relay_url'"
                ;;
            4)  # Query multiple allowed kinds
                kind2=$(get_random_kind)
                query_cmd="nak req -k $kind -k $kind2 -l 200 '$relay_url'"
                ;;
        esac
        
        # Use timeout and measure until all events are received
        if timeout 10s bash -c "$query_cmd 2>&1" > /tmp/nak_read_output 2>&1; then
            local end_time=$(get_time_ms)
            local latency=$((end_time - start_time))
            
            # Check if we got rate limited
            if grep -q "rate\|limit\|429\|too many" /tmp/nak_read_output 2>/dev/null; then
                ((rate_limited++))
                if [ "$VERBOSE" = true ]; then
                    echo "Read $i: Rate limited"
                fi
            else
                ((successful++))
                echo "$latency" >> "$temp_file"
                
                if [ "$VERBOSE" = true ]; then
                    echo "Read $i (type $query_type, kind $kind): ${latency}ms"
                fi
            fi
        else
            ((failed++))
            if [ "$VERBOSE" = true ]; then
                echo "Read $i: Failed or timed out"
            fi
        fi
        
        # Progress indicator
        if [ "$VERBOSE" = false ]; then
            printf "\r[%3d/%3d] Success: %3d, Failed: %3d, Rate limited: %3d" \
                "$i" "$NUM_READS" "$successful" "$failed" "$rate_limited"
        fi
        
        # Sleep between requests
        if [ $i -lt $NUM_READS ]; then
            sleep_ms "$REQUEST_DELAY"
        fi
    done
    
    echo ""
    
    # Store results
    if [ "$relay_name" = "relay1" ]; then
        RELAY1_READ_SUCCESS=$successful
        RELAY1_READ_FAILED=$failed
        RELAY1_READ_RATE_LIMITED=$rate_limited
    else
        RELAY2_READ_SUCCESS=$successful
        RELAY2_READ_FAILED=$failed
        RELAY2_READ_RATE_LIMITED=$rate_limited
    fi
    
    if [ -s "$temp_file" ] && [ $successful -gt 0 ]; then
        store_results "$relay_name" "read" "$temp_file"
        
        echo -e "\n${GREEN}Reading Results:${NC}"
        echo "  Successful: $successful / $NUM_READS"
        echo "  Failed/Timeout: $failed"
        echo "  Rate limited: $rate_limited"
    else
        echo -e "\n${RED}No successful reads to calculate statistics${NC}"
    fi
    
    rm -f "$temp_file" /tmp/nak_read_output
}

# Calculate percentage difference
calc_diff() {
    local val1=$1
    local val2=$2
    if (( $(echo "$val2 > 0" | bc -l) )); then
        echo "scale=1; (($val1 - $val2) / $val2) * 100" | bc
    else
        echo "0"
    fi
}

# Generate comparison summary
generate_comparison() {
    echo -e "\n${YELLOW}=== PERFORMANCE COMPARISON SUMMARY ===${NC}"
    echo -e "${CYAN}Relay 1:${NC} $RELAY1_URL"
    echo -e "${MAGENTA}Relay 2:${NC} $RELAY2_URL"
    echo ""
    
    # Publishing comparison
    echo -e "${YELLOW}PUBLISHING PERFORMANCE:${NC}"
    
    if [[ -n "${RELAY1_PUB_AVG:-}" ]] && [[ -n "${RELAY2_PUB_AVG:-}" ]]; then
        local pub_avg1=${RELAY1_PUB_AVG}
        local pub_avg2=${RELAY2_PUB_AVG}
        local pub_p50_1=${RELAY1_PUB_P50}
        local pub_p50_2=${RELAY2_PUB_P50}
        local pub_p95_1=${RELAY1_PUB_P95}
        local pub_p95_2=${RELAY2_PUB_P95}
        
        echo "  Average Latency:"
        printf "    ${CYAN}Relay 1:${NC} %.2f ms\n" $pub_avg1
        printf "    ${MAGENTA}Relay 2:${NC} %.2f ms\n" $pub_avg2
        
        local diff=$(calc_diff $pub_avg1 $pub_avg2)
        if (( $(echo "$pub_avg1 < $pub_avg2" | bc -l) )); then
            echo -e "    ${GREEN}↑ Relay 1 is ${diff#-}% faster${NC}"
        else
            echo -e "    ${GREEN}↑ Relay 2 is $diff% faster${NC}"
        fi
        
        echo ""
        echo "  Median (P50) Latency:"
        echo "    ${CYAN}Relay 1:${NC} $pub_p50_1 ms"
        echo "    ${MAGENTA}Relay 2:${NC} $pub_p50_2 ms"
        
        echo ""
        echo "  95th Percentile:"
        echo "    ${CYAN}Relay 1:${NC} $pub_p95_1 ms"
        echo "    ${MAGENTA}Relay 2:${NC} $pub_p95_2 ms"
    else
        echo "  ${RED}Insufficient data for comparison${NC}"
    fi
    
    echo ""
    echo -e "${YELLOW}READING PERFORMANCE:${NC}"
    
    if [[ -n "${RELAY1_READ_AVG:-}" ]] && [[ -n "${RELAY2_READ_AVG:-}" ]]; then
        local read_avg1=${RELAY1_READ_AVG}
        local read_avg2=${RELAY2_READ_AVG}
        local read_p50_1=${RELAY1_READ_P50}
        local read_p50_2=${RELAY2_READ_P50}
        local read_p95_1=${RELAY1_READ_P95}
        local read_p95_2=${RELAY2_READ_P95}
        
        echo "  Average Latency (200 events):"
        printf "    ${CYAN}Relay 1:${NC} %.2f ms\n" $read_avg1
        printf "    ${MAGENTA}Relay 2:${NC} %.2f ms\n" $read_avg2
        
        local diff=$(calc_diff $read_avg1 $read_avg2)
        if (( $(echo "$read_avg1 < $read_avg2" | bc -l) )); then
            echo -e "    ${GREEN}↑ Relay 1 is ${diff#-}% faster${NC}"
        else
            echo -e "    ${GREEN}↑ Relay 2 is $diff% faster${NC}"
        fi
        
        echo ""
        echo "  Median (P50) Latency:"
        echo "    ${CYAN}Relay 1:${NC} $read_p50_1 ms"
        echo "    ${MAGENTA}Relay 2:${NC} $read_p50_2 ms"
        
        echo ""
        echo "  95th Percentile:"
        echo "    ${CYAN}Relay 1:${NC} $read_p95_1 ms"
        echo "    ${MAGENTA}Relay 2:${NC} $read_p95_2 ms"
    else
        echo "  ${RED}Insufficient data for comparison${NC}"
    fi
    
    echo ""
    echo -e "${YELLOW}RELIABILITY:${NC}"
    echo "  Publishing Success Rate:"
    echo "    ${CYAN}Relay 1:${NC} ${RELAY1_PUB_SUCCESS:-0}/${NUM_EVENTS}"
    echo "    ${MAGENTA}Relay 2:${NC} ${RELAY2_PUB_SUCCESS:-0}/${NUM_EVENTS}"
    
    echo "  Reading Success Rate:"
    echo "    ${CYAN}Relay 1:${NC} ${RELAY1_READ_SUCCESS:-0}/${NUM_READS}"
    echo "    ${MAGENTA}Relay 2:${NC} ${RELAY2_READ_SUCCESS:-0}/${NUM_READS}"
    
    echo ""
    echo -e "${GREEN}Lower latency values indicate better performance${NC}"
    echo "Test completed at: $(date)"
}

# Main execution
main() {
    parse_args "$@"
    check_dependencies
    
    echo -e "${BLUE}Starting Groups Relay Performance Comparison${NC}"
    echo "Relay 1: $RELAY1_URL"
    echo "Relay 2: $RELAY2_URL"
    echo ""
    
    setup_keys
    
    # Test Relay 1
    echo -e "\n${CYAN}>>> Testing Relay 1: $RELAY1_URL${NC}"
    test_event_publishing "$RELAY1_URL" "relay1" "$CYAN"
    test_event_reading "$RELAY1_URL" "relay1" "$CYAN"
    
    # Test Relay 2
    echo -e "\n${MAGENTA}>>> Testing Relay 2: $RELAY2_URL${NC}"
    test_event_publishing "$RELAY2_URL" "relay2" "$MAGENTA"
    test_event_reading "$RELAY2_URL" "relay2" "$MAGENTA"
    
    # Generate comparison summary
    generate_comparison
}

# Run the script
main "$@"