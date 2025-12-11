#!/bin/bash
# Quick performance test for TrafficCop
# Usage: ./perf-test.sh [concurrency] [duration]

CONCURRENCY=${1:-300}
DURATION=${2:-30s}

cd "$(dirname "$0")"

echo "Running TrafficCop benchmark..."
echo "  Concurrency: $CONCURRENCY"
echo "  Duration: $DURATION"
echo ""

CONCURRENCY=$CONCURRENCY DURATION=$DURATION ./bench.sh --simple-progress
