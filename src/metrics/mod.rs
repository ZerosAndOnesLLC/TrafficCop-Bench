#![allow(dead_code)]

use hdrhistogram::Histogram;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

/// Tracks latency metrics using HDR Histogram for accurate percentile calculations
/// Uses a Mutex instead of RwLock since we always need write access for recording
pub struct LatencyHistogram {
    histogram: Mutex<Histogram<u64>>,
}

impl LatencyHistogram {
    pub fn new() -> Self {
        // Track latencies from 1 microsecond to 60 seconds with 3 significant figures
        let histogram = Histogram::new_with_bounds(1, 60_000_000, 3)
            .expect("Failed to create histogram");
        Self {
            histogram: Mutex::new(histogram),
        }
    }

    #[inline]
    pub async fn record(&self, duration: Duration) {
        let micros = duration.as_micros() as u64;
        // Use try_lock to avoid blocking - if we can't get the lock, skip this sample
        // This trades a tiny bit of accuracy for much better throughput
        if let Ok(mut hist) = self.histogram.try_lock() {
            let _ = hist.record(micros);
        }
    }

    pub async fn percentile(&self, p: f64) -> Duration {
        let hist = self.histogram.lock().await;
        Duration::from_micros(hist.value_at_percentile(p))
    }

    pub async fn mean(&self) -> Duration {
        let hist = self.histogram.lock().await;
        Duration::from_micros(hist.mean() as u64)
    }

    pub async fn min(&self) -> Duration {
        let hist = self.histogram.lock().await;
        Duration::from_micros(hist.min())
    }

    pub async fn max(&self) -> Duration {
        let hist = self.histogram.lock().await;
        Duration::from_micros(hist.max())
    }

    pub async fn count(&self) -> u64 {
        let hist = self.histogram.lock().await;
        hist.len()
    }

    pub async fn reset(&self) {
        let mut hist = self.histogram.lock().await;
        hist.reset();
    }

    pub async fn snapshot(&self) -> LatencySnapshot {
        let hist = self.histogram.lock().await;
        LatencySnapshot {
            count: hist.len(),
            min: Duration::from_micros(hist.min()),
            max: Duration::from_micros(hist.max()),
            mean: Duration::from_micros(hist.mean() as u64),
            p50: Duration::from_micros(hist.value_at_percentile(50.0)),
            p75: Duration::from_micros(hist.value_at_percentile(75.0)),
            p90: Duration::from_micros(hist.value_at_percentile(90.0)),
            p95: Duration::from_micros(hist.value_at_percentile(95.0)),
            p99: Duration::from_micros(hist.value_at_percentile(99.0)),
            p999: Duration::from_micros(hist.value_at_percentile(99.9)),
        }
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of latency statistics
#[derive(Debug, Clone)]
pub struct LatencySnapshot {
    pub count: u64,
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
    pub p50: Duration,
    pub p75: Duration,
    pub p90: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub p999: Duration,
}

/// Atomic counters for request tracking
#[derive(Debug)]
pub struct RequestCounters {
    pub total: AtomicU64,
    pub success: AtomicU64,
    pub errors: AtomicU64,
    pub timeouts: AtomicU64,
    pub connection_errors: AtomicU64,
}

impl RequestCounters {
    pub fn new() -> Self {
        Self {
            total: AtomicU64::new(0),
            success: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            connection_errors: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn record_success(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.success.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_success(&self, count: u64) {
        self.total.fetch_add(count, Ordering::Relaxed);
        self.success.fetch_add(count, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_error(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_timeout(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_connection_error(&self) {
        self.connection_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            total: self.total.load(Ordering::Relaxed),
            success: self.success.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            connection_errors: self.connection_errors.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
        self.success.store(0, Ordering::Relaxed);
        self.errors.store(0, Ordering::Relaxed);
        self.timeouts.store(0, Ordering::Relaxed);
        self.connection_errors.store(0, Ordering::Relaxed);
    }
}

impl Default for RequestCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of counter values
#[derive(Debug, Clone)]
pub struct CounterSnapshot {
    pub total: u64,
    pub success: u64,
    pub errors: u64,
    pub timeouts: u64,
    pub connection_errors: u64,
}

impl CounterSnapshot {
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.success as f64 / self.total as f64) * 100.0
        }
    }

    pub fn error_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.errors as f64 / self.total as f64) * 100.0
        }
    }
}

/// Tracks bytes sent and received
#[derive(Debug)]
pub struct BytesCounter {
    pub sent: AtomicU64,
    pub received: AtomicU64,
}

impl BytesCounter {
    pub fn new() -> Self {
        Self {
            sent: AtomicU64::new(0),
            received: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn record_sent(&self, bytes: u64) {
        self.sent.fetch_add(bytes, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_received(&self, bytes: u64) {
        self.received.fetch_add(bytes, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_received(&self, bytes: u64) {
        self.received.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> BytesSnapshot {
        BytesSnapshot {
            sent: self.sent.load(Ordering::Relaxed),
            received: self.received.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.sent.store(0, Ordering::Relaxed);
        self.received.store(0, Ordering::Relaxed);
    }
}

impl Default for BytesCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of bytes transferred
#[derive(Debug, Clone)]
pub struct BytesSnapshot {
    pub sent: u64,
    pub received: u64,
}

/// Tracks HTTP status code distribution
pub struct StatusCodeTracker {
    codes: RwLock<std::collections::HashMap<u16, u64>>,
}

impl StatusCodeTracker {
    pub fn new() -> Self {
        Self {
            codes: RwLock::new(std::collections::HashMap::new()),
        }
    }

    #[inline]
    pub async fn record(&self, status: u16) {
        // Use try_write to avoid blocking
        if let Ok(mut codes) = self.codes.try_write() {
            *codes.entry(status).or_insert(0) += 1;
        }
    }

    #[inline]
    pub async fn add(&self, status: u16, count: u64) {
        if let Ok(mut codes) = self.codes.try_write() {
            *codes.entry(status).or_insert(0) += count;
        }
    }

    pub async fn snapshot(&self) -> std::collections::HashMap<u16, u64> {
        self.codes.read().await.clone()
    }

    pub async fn reset(&self) {
        self.codes.write().await.clear();
    }
}

impl Default for StatusCodeTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Combined metrics collector
pub struct MetricsCollector {
    pub latency: LatencyHistogram,
    pub counters: RequestCounters,
    pub bytes: BytesCounter,
    pub status_codes: StatusCodeTracker,
    pub start_time: Instant,
    pub test_name: String,
}

impl MetricsCollector {
    pub fn new(test_name: impl Into<String>) -> Self {
        Self {
            latency: LatencyHistogram::new(),
            counters: RequestCounters::new(),
            bytes: BytesCounter::new(),
            status_codes: StatusCodeTracker::new(),
            start_time: Instant::now(),
            test_name: test_name.into(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub async fn requests_per_second(&self) -> f64 {
        let elapsed = self.elapsed().as_secs_f64();
        if elapsed == 0.0 {
            0.0
        } else {
            self.counters.snapshot().total as f64 / elapsed
        }
    }

    pub async fn full_snapshot(&self) -> MetricsSnapshot {
        let counters = self.counters.snapshot();
        let elapsed = self.elapsed();
        let rps = if elapsed.as_secs_f64() == 0.0 {
            0.0
        } else {
            counters.total as f64 / elapsed.as_secs_f64()
        };

        MetricsSnapshot {
            test_name: self.test_name.clone(),
            duration: elapsed,
            counters,
            latency: self.latency.snapshot().await,
            bytes: self.bytes.snapshot(),
            status_codes: self.status_codes.snapshot().await,
            requests_per_second: rps,
        }
    }

    pub async fn reset(&self) {
        self.latency.reset().await;
        self.counters.reset();
        self.bytes.reset();
        self.status_codes.reset().await;
    }
}

/// Complete metrics snapshot for reporting
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub test_name: String,
    pub duration: Duration,
    pub counters: CounterSnapshot,
    pub latency: LatencySnapshot,
    pub bytes: BytesSnapshot,
    pub status_codes: std::collections::HashMap<u16, u64>,
    pub requests_per_second: f64,
}

/// Thread-safe metrics collector wrapper
pub type SharedMetrics = Arc<MetricsCollector>;

pub fn new_shared_metrics(test_name: impl Into<String>) -> SharedMetrics {
    Arc::new(MetricsCollector::new(test_name))
}
