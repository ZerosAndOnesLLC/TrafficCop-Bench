#!/bin/bash
#
# TrafficCop Benchmark Script
# Convenience wrapper for trafficcop-bench bench command
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRAFFICCOP_DIR="/home/mack/dev/traffic_management"
TRAFFICCOP_BIN="$TRAFFICCOP_DIR/target/release/traffic_management"
TRAFFICCOP_CONFIG="$SCRIPT_DIR/config/bench-local.yaml"
BENCH_BIN="$SCRIPT_DIR/target/release/trafficcop-bench"

# Default test parameters (can be overridden via env vars)
CONCURRENCY=${CONCURRENCY:-100}
DURATION=${DURATION:-30s}
WARMUP=${WARMUP:-5s}

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if binaries need to be built
if [ ! -x "$TRAFFICCOP_BIN" ]; then
    log_warn "TrafficCop release binary not found"
    log_info "Building TrafficCop (release)..."
    cd "$TRAFFICCOP_DIR"
    cargo build --release
    cd - > /dev/null
fi

if [ ! -x "$BENCH_BIN" ]; then
    log_warn "Bench binary not found"
    log_info "Building trafficcop-bench (release)..."
    cd "$SCRIPT_DIR"
    cargo build --release
    cd - > /dev/null
fi

# Run the integrated benchmark (all pure Rust, in-memory)
exec "$BENCH_BIN" bench \
    --trafficcop "$TRAFFICCOP_BIN" \
    --config "$TRAFFICCOP_CONFIG" \
    -c "$CONCURRENCY" \
    -d "$DURATION" \
    --warmup "$WARMUP" \
    "$@"
