# TrafficCop Bench

High-performance load testing tool for TrafficCop reverse proxy. Supports HTTP, WebSocket, gRPC, and TCP protocols with real-time TUI display and detailed metrics.

## Features

- **Multi-Protocol Support**: HTTP/HTTPS, WebSocket, gRPC, and TCP
- **Real-time TUI Dashboard**: Live visualization of throughput, latency percentiles, and status codes
- **Accurate Latency Metrics**: HDR Histogram for precise percentile calculations (p50, p90, p95, p99, p99.9)
- **Configurable Load Patterns**: Duration-based or request-count-based tests with rate limiting
- **Warm-up Phase**: Optional warm-up period before collecting metrics
- **Multiple Output Formats**: Text, JSON, and CSV export
- **Configuration Files**: YAML-based configuration for complex test scenarios
- **Result Comparison**: Compare test results to track performance changes

## Installation

```bash
cd trafficcop-bench
cargo build --release
```

The binary will be at `target/release/trafficcop-bench`.

## Quick Start

### HTTP Load Test

```bash
# Basic HTTP GET test (10 concurrent connections, 30 seconds)
trafficcop-bench http http://localhost:8080 -c 10 -d 30s

# HTTP POST with body
trafficcop-bench http http://localhost:8080/api -c 50 -d 1m -m POST -b '{"key":"value"}'

# With rate limiting (1000 req/s)
trafficcop-bench http http://localhost:8080 -c 100 -d 30s -r 1000

# Output to JSON file
trafficcop-bench http http://localhost:8080 -c 10 -d 30s -o json --output-file results.json
```

### WebSocket Load Test

```bash
# WebSocket test (10 connections, 10 messages each)
trafficcop-bench ws ws://localhost:8080/ws -c 10 -d 30s --messages 10

# With custom message
trafficcop-bench ws ws://localhost:8080/ws -c 10 -d 30s -m "ping"
```

### gRPC Load Test

```bash
# gRPC health check
trafficcop-bench grpc localhost:50051 -c 10 -d 30s

# Custom service/method
trafficcop-bench grpc localhost:50051 -c 10 -d 30s --service mypackage.MyService --method MyMethod
```

### TCP Load Test

```bash
# Raw TCP connection test
trafficcop-bench tcp localhost:6379 -c 10 -d 30s

# With data to send
trafficcop-bench tcp localhost:6379 -c 10 -d 30s --send "PING\r\n" --expect "PONG"
```

## CLI Reference

### Common Options

| Option | Description |
|--------|-------------|
| `-c, --concurrency` | Number of concurrent connections (default: 10) |
| `-d, --duration` | Test duration (e.g., "30s", "5m", "1h") |
| `-n, --requests` | Total number of requests (alternative to duration) |
| `-r, --rate` | Target requests per second (0 = unlimited) |
| `--timeout` | Request timeout (default: "30s") |
| `--warmup` | Warm-up duration before collecting metrics |
| `-o, --output` | Output format: text, json, csv |
| `--output-file` | Write results to file |
| `--no-progress` | Disable real-time progress display |
| `--simple-progress` | Use simple text output instead of TUI |
| `--name` | Test name for reports |

### HTTP-specific Options

| Option | Description |
|--------|-------------|
| `-m, --method` | HTTP method (default: GET) |
| `-b, --body` | Request body |
| `-H, --header` | Headers (can be repeated, format: "Key: Value") |
| `--insecure` | Skip TLS certificate verification |
| `--no-http2` | Disable HTTP/2 |

### WebSocket-specific Options

| Option | Description |
|--------|-------------|
| `--messages` | Messages per connection (default: 10) |
| `-m, --message` | Message payload |
| `--message-size` | Random message size in bytes (default: 256) |

### gRPC-specific Options

| Option | Description |
|--------|-------------|
| `--service` | gRPC service name |
| `--method` | gRPC method name |
| `--request` | Request payload as JSON |
| `--tls` | Enable TLS |

### TCP-specific Options

| Option | Description |
|--------|-------------|
| `--send` | Data to send |
| `--send-hex` | Hex-encoded data to send |
| `--expect` | Expected response string |
| `--tls` | Enable TLS |
| `--insecure` | Skip TLS verification |

## Configuration File

Create a YAML configuration file for complex test scenarios:

```yaml
name: "api-load-test"
target: "http://localhost:8080"
duration: "5m"
concurrency: 100
rate: 5000
warmup: "30s"
timeout: "30s"

http:
  method: "POST"
  headers:
    Authorization: "Bearer token123"
    Content-Type: "application/json"
  body: '{"action":"test"}'
  http2: true
  insecure: false
  expected_status: [200, 201]

output:
  format: "json"
  file: "results.json"
  progress: true
  progress_interval: "1s"
```

Run with:

```bash
trafficcop-bench run config.yaml
```

## Output Example

```
═══════════════════════════════════════════════════════════════════════════════
                         LOAD TEST RESULTS
═══════════════════════════════════════════════════════════════════════════════

Test:        http-load-test
Target:      http://localhost:8080
Protocol:    HTTP
Duration:    30.00s
Concurrency: 100

───────────────────────────────────────────────────────────────────────────────
                              THROUGHPUT
───────────────────────────────────────────────────────────────────────────────
Requests/sec:     45672.31
Total Requests:   1370169
Successful:       1370169
Failed:           0
Timeouts:         0
Conn Errors:      0
Success Rate:     100.00%

───────────────────────────────────────────────────────────────────────────────
                              LATENCY
───────────────────────────────────────────────────────────────────────────────
Min:              0.12ms
Max:              45.23ms
Mean:             2.18ms
p50:              1.95ms
p75:              2.45ms
p90:              3.21ms
p95:              4.12ms
p99:              8.45ms
p99.9:            23.67ms

───────────────────────────────────────────────────────────────────────────────
                              DATA TRANSFER
───────────────────────────────────────────────────────────────────────────────
Bytes Sent:       123.45 MB
Bytes Received:   678.90 MB
Throughput:       26.74 MB/s

───────────────────────────────────────────────────────────────────────────────
                           STATUS CODE DISTRIBUTION
───────────────────────────────────────────────────────────────────────────────
HTTP 200:         1370169 (100.0%)

═══════════════════════════════════════════════════════════════════════════════
```

## Comparing Results

Compare two test runs to track performance changes:

```bash
trafficcop-bench compare baseline.json current.json
```

Output:

```
═══════════════════════════════════════════════════════════════════════════════
                         COMPARISON: baseline vs current
═══════════════════════════════════════════════════════════════════════════════

Requests/sec:  +15.23%
p50 Latency:   -8.45%
p99 Latency:   -12.30%
Error Rate:    -100.00%

Overall: IMPROVEMENT ✓
═══════════════════════════════════════════════════════════════════════════════
```

## Architecture

```
trafficcop-bench/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── lib.rs           # Library exports
│   ├── config/          # Configuration parsing
│   ├── http/            # HTTP load testing
│   ├── websocket/       # WebSocket load testing
│   ├── grpc/            # gRPC load testing
│   ├── tcp/             # TCP load testing
│   ├── metrics/         # HDR Histogram metrics
│   ├── display/         # TUI display
│   └── report/          # Report generation
```

## Performance Tips

1. **Use release builds**: `cargo build --release`
2. **Disable TUI for maximum throughput**: `--no-progress`
3. **Tune OS limits**: Increase file descriptor limits for high concurrency
4. **Use connection pooling**: HTTP tests use keep-alive by default
5. **Warm-up phase**: Use `--warmup` to prime connections before measuring

## License

MIT
