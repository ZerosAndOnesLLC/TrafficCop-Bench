use crate::config::BenchConfig;
use crate::metrics::SharedMetrics;
use reqwest::{Client, Method, Response};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// HTTP load tester
pub struct HttpTester {
    client: Client,
    config: BenchConfig,
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    request_count: Arc<AtomicU64>,
}

impl HttpTester {
    pub fn new(config: BenchConfig, metrics: SharedMetrics) -> Result<Self, HttpError> {
        let mut client_builder = Client::builder()
            .timeout(config.timeout)
            .pool_max_idle_per_host(config.concurrency as usize)
            .tcp_keepalive(Duration::from_secs(60));

        if config.http.insecure {
            client_builder = client_builder.danger_accept_invalid_certs(true);
        }

        if !config.http.http2 {
            client_builder = client_builder.http1_only();
        }

        let client = client_builder
            .build()
            .map_err(|e| HttpError::ClientBuild(e.to_string()))?;

        Ok(Self {
            client,
            config,
            metrics,
            running: Arc::new(AtomicBool::new(false)),
            request_count: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Run the load test
    pub async fn run(&self) -> Result<(), HttpError> {
        self.running.store(true, Ordering::SeqCst);
        self.request_count.store(0, Ordering::SeqCst);

        info!(
            "Starting HTTP load test against {} with {} concurrent connections",
            self.config.target, self.config.concurrency
        );

        // Warm-up phase if configured
        if let Some(warmup) = self.config.warmup {
            info!("Starting warm-up phase for {:?}", warmup);
            self.run_warmup(warmup).await?;
            self.metrics.reset().await;
            info!("Warm-up complete, starting measurement");
        }

        // Rate limiter setup
        let rate_limiter = if self.config.rate > 0 {
            Some(RateLimiter::new(self.config.rate))
        } else {
            None
        };

        // Semaphore for concurrency control
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));

        // Determine test end condition
        let test_duration = self.config.duration;
        let test_requests = self.config.requests;
        let start_time = Instant::now();

        let mut handles = Vec::new();

        // Spawn worker tasks
        for _ in 0..self.config.concurrency {
            let client = self.client.clone();
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let running = Arc::clone(&self.running);
            let request_count = Arc::clone(&self.request_count);
            let semaphore = Arc::clone(&semaphore);
            let rate_limiter = rate_limiter.clone();

            let handle = tokio::spawn(async move {
                loop {
                    // Check termination conditions
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }

                    if let Some(duration) = test_duration {
                        if start_time.elapsed() >= duration {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }

                    if let Some(max_requests) = test_requests {
                        let current = request_count.fetch_add(1, Ordering::SeqCst);
                        if current >= max_requests {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }

                    // Rate limiting
                    if let Some(ref limiter) = rate_limiter {
                        limiter.wait().await;
                    }

                    // Acquire semaphore permit
                    let _permit = semaphore.acquire().await.unwrap();

                    // Execute request
                    let result =
                        execute_request(&client, &config, &metrics).await;

                    if let Err(e) = result {
                        debug!("Request error: {:?}", e);
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all workers to complete
        for handle in handles {
            let _ = handle.await;
        }

        self.running.store(false, Ordering::SeqCst);
        info!("HTTP load test complete");

        Ok(())
    }

    async fn run_warmup(&self, duration: Duration) -> Result<(), HttpError> {
        let start = Instant::now();
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));

        while start.elapsed() < duration {
            let client = self.client.clone();
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let semaphore = Arc::clone(&semaphore);

            let _permit = semaphore.acquire().await.unwrap();

            tokio::spawn(async move {
                let _ = execute_request(&client, &config, &metrics).await;
            });
        }

        Ok(())
    }

    /// Stop the test
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Check if test is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Execute a single HTTP request and record metrics
async fn execute_request(
    client: &Client,
    config: &BenchConfig,
    metrics: &SharedMetrics,
) -> Result<Response, HttpError> {
    let method: Method = config
        .http
        .method
        .parse()
        .map_err(|_| HttpError::InvalidMethod(config.http.method.clone()))?;

    let mut request = client.request(method, &config.target);

    // Add headers
    for (key, value) in &config.http.headers {
        request = request.header(key.as_str(), value.as_str());
    }

    // Add content-type if specified
    if let Some(ref content_type) = config.http.content_type {
        request = request.header("Content-Type", content_type.as_str());
    }

    // Add body if present
    if let Some(ref body) = config.http.body {
        request = request.body(body.clone());
        metrics.bytes.record_sent(body.len() as u64);
    }

    let start = Instant::now();
    let result = request.send().await;
    let latency = start.elapsed();

    match result {
        Ok(response) => {
            let status = response.status().as_u16();
            metrics.status_codes.record(status).await;
            metrics.latency.record(latency).await;

            // Check if status is expected
            if config.http.expected_status.contains(&status) {
                metrics.counters.record_success();
            } else {
                metrics.counters.record_error();
                warn!("Unexpected status code: {}", status);
            }

            // Read response body to get bytes received
            match response.bytes().await {
                Ok(bytes) => {
                    metrics.bytes.record_received(bytes.len() as u64);
                }
                Err(e) => {
                    debug!("Error reading response body: {:?}", e);
                }
            }

            Ok(reqwest::Response::from(
                http::Response::builder()
                    .status(status)
                    .body(reqwest::Body::default())
                    .unwrap(),
            ))
        }
        Err(e) => {
            if e.is_timeout() {
                metrics.counters.record_timeout();
                Err(HttpError::Timeout)
            } else if e.is_connect() {
                metrics.counters.record_connection_error();
                Err(HttpError::Connection(e.to_string()))
            } else {
                metrics.counters.record_error();
                Err(HttpError::Request(e.to_string()))
            }
        }
    }
}

/// Simple token bucket rate limiter
#[derive(Clone)]
struct RateLimiter {
    interval: Duration,
}

impl RateLimiter {
    fn new(rate: u64) -> Self {
        Self {
            interval: Duration::from_secs_f64(1.0 / rate as f64),
        }
    }

    async fn wait(&self) {
        sleep(self.interval).await;
    }
}

#[derive(Debug)]
pub enum HttpError {
    ClientBuild(String),
    InvalidMethod(String),
    Request(String),
    Connection(String),
    Timeout,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::ClientBuild(e) => write!(f, "Failed to build HTTP client: {}", e),
            HttpError::InvalidMethod(m) => write!(f, "Invalid HTTP method: {}", m),
            HttpError::Request(e) => write!(f, "Request error: {}", e),
            HttpError::Connection(e) => write!(f, "Connection error: {}", e),
            HttpError::Timeout => write!(f, "Request timeout"),
        }
    }
}

impl std::error::Error for HttpError {}

/// Multi-endpoint HTTP tester for testing multiple paths
pub struct MultiEndpointTester {
    testers: Vec<HttpTester>,
}

impl MultiEndpointTester {
    pub fn new(
        base_config: BenchConfig,
        metrics: SharedMetrics,
    ) -> Result<Self, HttpError> {
        let endpoints = &base_config.http.endpoints;

        if endpoints.is_empty() {
            // Single endpoint test
            let tester = HttpTester::new(base_config, metrics)?;
            return Ok(Self {
                testers: vec![tester],
            });
        }

        // Create a tester for each endpoint with weighted distribution
        let mut testers = Vec::new();

        for endpoint in endpoints {
            let mut config = base_config.clone();

            // Build full URL
            if endpoint.path.starts_with("http") {
                config.target = endpoint.path.clone();
            } else {
                config.target = format!("{}{}", base_config.target.trim_end_matches('/'), endpoint.path);
            }

            // Override method if specified
            if let Some(ref method) = endpoint.method {
                config.http.method = method.clone();
            }

            // Override body if specified
            if endpoint.body.is_some() {
                config.http.body = endpoint.body.clone();
            }

            // Merge headers
            for (k, v) in &endpoint.headers {
                config.http.headers.insert(k.clone(), v.clone());
            }

            // Adjust concurrency based on weight
            let total_weight: u32 = endpoints.iter().map(|e| e.weight).sum();
            config.concurrency = (base_config.concurrency * endpoint.weight / total_weight).max(1);

            let tester = HttpTester::new(config, Arc::clone(&metrics))?;
            testers.push(tester);
        }

        Ok(Self { testers })
    }

    pub async fn run(&self) -> Result<(), HttpError> {
        let handles: Vec<_> = self
            .testers
            .iter()
            .map(|t| {
                let tester = t.clone();
                tokio::spawn(async move { tester.run().await })
            })
            .collect();

        for handle in handles {
            handle.await.map_err(|e| HttpError::Request(e.to_string()))??;
        }

        Ok(())
    }

    pub fn stop(&self) {
        for tester in &self.testers {
            tester.stop();
        }
    }
}

impl Clone for HttpTester {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            config: self.config.clone(),
            metrics: Arc::clone(&self.metrics),
            running: Arc::clone(&self.running),
            request_count: Arc::clone(&self.request_count),
        }
    }
}
