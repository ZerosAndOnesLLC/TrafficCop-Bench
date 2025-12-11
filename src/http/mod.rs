#![allow(dead_code)]

use crate::config::BenchConfig;
use crate::metrics::SharedMetrics;
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::sleep;
use tracing::info;
use url::Url;

// Pre-allocated static request parts
static CONNECTION_KEEP_ALIVE: &str = "keep-alive";

/// HTTP load tester using hyper with pipelining for maximum performance
pub struct HttpTester {
    config: BenchConfig,
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    request_count: Arc<AtomicU64>,
    parsed_url: Url,
}

impl Clone for HttpTester {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            metrics: Arc::clone(&self.metrics),
            running: Arc::clone(&self.running),
            request_count: Arc::clone(&self.request_count),
            parsed_url: self.parsed_url.clone(),
        }
    }
}

impl HttpTester {
    pub fn new(config: BenchConfig, metrics: SharedMetrics) -> Result<Self, HttpError> {
        let parsed_url =
            Url::parse(&config.target).map_err(|e| HttpError::InvalidUrl(e.to_string()))?;

        Ok(Self {
            config,
            metrics,
            running: Arc::new(AtomicBool::new(false)),
            request_count: Arc::new(AtomicU64::new(0)),
            parsed_url,
        })
    }

    pub async fn run(&self) -> Result<(), HttpError> {
        self.running.store(true, Ordering::SeqCst);
        self.request_count.store(0, Ordering::SeqCst);

        info!(
            "Starting HTTP load test against {} with {} concurrent connections",
            self.config.target, self.config.concurrency
        );

        // Warm-up phase
        if let Some(warmup) = self.config.warmup {
            info!("Starting warm-up phase for {:?}", warmup);
            self.run_phase(warmup).await?;
            self.metrics.reset().await;
            info!("Warm-up complete, starting measurement");
        }

        // Main test phase
        if let Some(duration) = self.config.duration {
            self.run_phase(duration).await?;
        }

        self.running.store(false, Ordering::SeqCst);
        info!("HTTP load test complete");

        Ok(())
    }

    async fn run_phase(&self, duration: Duration) -> Result<(), HttpError> {
        let start = Instant::now();
        let mut handles = Vec::with_capacity(self.config.concurrency as usize);

        let host: Arc<str> = self.parsed_url.host_str().unwrap_or("localhost").into();
        let port = self.parsed_url.port().unwrap_or(80);
        let addr: Arc<str> = format!("{}:{}", host, port).into();
        let path: Arc<str> = {
            let p = self.parsed_url.path();
            if p.is_empty() { "/".into() } else { p.into() }
        };

        // Spawn connection workers
        for _ in 0..self.config.concurrency {
            let metrics = Arc::clone(&self.metrics);
            let running = Arc::clone(&self.running);
            let addr = Arc::clone(&addr);
            let path = Arc::clone(&path);
            let host = Arc::clone(&host);

            let handle = tokio::spawn(async move {
                // Local counters to batch updates
                let mut local_success: u64 = 0;
                let mut local_bytes: u64 = 0;
                const BATCH_SIZE: u64 = 100;

                loop {
                    if !running.load(Ordering::Relaxed) || start.elapsed() >= duration {
                        break;
                    }

                    // Connect with optimized settings
                    let stream = match TcpStream::connect(addr.as_ref()).await {
                        Ok(s) => s,
                        Err(_) => {
                            metrics.counters.record_connection_error();
                            continue;
                        }
                    };

                    // Optimize TCP settings
                    stream.set_nodelay(true).ok();

                    let io = TokioIo::new(stream);
                    let (mut sender, conn) = match hyper::client::conn::http1::Builder::new()
                        .handshake(io)
                        .await
                    {
                        Ok(h) => h,
                        Err(_) => continue,
                    };

                    // Drive connection
                    tokio::spawn(async move {
                        let _ = conn.await;
                    });

                    // Fire requests as fast as possible with pipelining
                    while running.load(Ordering::Relaxed) && start.elapsed() < duration {
                        let req_start = Instant::now();

                        // Build minimal request
                        let req = Request::builder()
                            .method(Method::GET)
                            .uri(path.as_ref())
                            .header("host", host.as_ref())
                            .header("connection", CONNECTION_KEEP_ALIVE)
                            .body(Empty::<Bytes>::new())
                            .unwrap();

                        match sender.send_request(req).await {
                            Ok(resp) => {
                                let status = resp.status().as_u16();

                                // Consume body as fast as possible
                                match resp.into_body().collect().await {
                                    Ok(body) => {
                                        let latency = req_start.elapsed();
                                        let bytes = body.to_bytes().len() as u64;

                                        // Batch local counters
                                        local_success += 1;
                                        local_bytes += bytes;

                                        // Record latency (uses try_lock internally)
                                        metrics.latency.record(latency).await;

                                        // Flush batch periodically
                                        if local_success >= BATCH_SIZE {
                                            metrics.counters.add_success(local_success);
                                            metrics.bytes.add_received(local_bytes);
                                            metrics.status_codes.add(status, local_success).await;
                                            local_success = 0;
                                            local_bytes = 0;
                                        }
                                    }
                                    Err(_) => {
                                        metrics.counters.record_error();
                                    }
                                }
                            }
                            Err(_) => {
                                // Flush remaining before reconnect
                                if local_success > 0 {
                                    metrics.counters.add_success(local_success);
                                    metrics.bytes.add_received(local_bytes);
                                    local_success = 0;
                                    local_bytes = 0;
                                }
                                metrics.counters.record_connection_error();
                                break;
                            }
                        }
                    }
                }

                // Flush any remaining
                if local_success > 0 {
                    metrics.counters.add_success(local_success);
                    metrics.bytes.add_received(local_bytes);
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        Ok(())
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Rate limiter for controlling request rate
#[derive(Clone)]
pub struct RateLimiter {
    interval: Duration,
    last_request: Arc<tokio::sync::Mutex<Instant>>,
}

impl RateLimiter {
    pub fn new(requests_per_second: u64) -> Self {
        Self {
            interval: Duration::from_secs_f64(1.0 / requests_per_second as f64),
            last_request: Arc::new(tokio::sync::Mutex::new(Instant::now())),
        }
    }

    pub async fn wait(&self) {
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed();
        if elapsed < self.interval {
            sleep(self.interval - elapsed).await;
        }
        *last = Instant::now();
    }
}

/// Multi-endpoint tester
pub struct MultiEndpointTester {
    testers: Vec<(HttpTester, f64)>,
    metrics: SharedMetrics,
}

impl MultiEndpointTester {
    pub fn new(
        endpoints: Vec<(String, f64)>,
        config: BenchConfig,
        metrics: SharedMetrics,
    ) -> Result<Self, HttpError> {
        let mut testers = Vec::new();
        for (url, weight) in endpoints {
            let mut endpoint_config = config.clone();
            endpoint_config.target = url;
            let tester = HttpTester::new(endpoint_config, Arc::clone(&metrics))?;
            testers.push((tester, weight));
        }
        Ok(Self { testers, metrics })
    }

    pub async fn run(&self) -> Result<(), HttpError> {
        let mut handles = Vec::new();
        for (tester, _) in &self.testers {
            let tester = tester.clone();
            handles.push(tokio::spawn(async move { tester.run().await }));
        }
        for handle in handles {
            handle.await.map_err(|e| HttpError::Join(e.to_string()))??;
        }
        Ok(())
    }

    pub fn stop(&self) {
        for (tester, _) in &self.testers {
            tester.stop();
        }
    }
}

#[derive(Debug)]
pub enum HttpError {
    InvalidUrl(String),
    InvalidMethod(String),
    ClientBuild(String),
    Request(String),
    Timeout,
    Connection(String),
    Join(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::InvalidUrl(e) => write!(f, "Invalid URL: {}", e),
            HttpError::InvalidMethod(m) => write!(f, "Invalid HTTP method: {}", m),
            HttpError::ClientBuild(e) => write!(f, "Failed to build HTTP client: {}", e),
            HttpError::Request(e) => write!(f, "Request error: {}", e),
            HttpError::Timeout => write!(f, "Request timed out"),
            HttpError::Connection(e) => write!(f, "Connection error: {}", e),
            HttpError::Join(e) => write!(f, "Task join error: {}", e),
        }
    }
}

impl std::error::Error for HttpError {}
