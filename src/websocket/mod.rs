use crate::config::BenchConfig;
use crate::metrics::SharedMetrics;
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::Message,
};
use tracing::{debug, info};
use url::Url;

/// WebSocket load tester
pub struct WebSocketTester {
    config: BenchConfig,
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    connection_count: Arc<AtomicU64>,
}

impl WebSocketTester {
    pub fn new(config: BenchConfig, metrics: SharedMetrics) -> Self {
        Self {
            config,
            metrics,
            running: Arc::new(AtomicBool::new(false)),
            connection_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Run the WebSocket load test
    pub async fn run(&self) -> Result<(), WebSocketError> {
        self.running.store(true, Ordering::SeqCst);
        self.connection_count.store(0, Ordering::SeqCst);

        // Convert HTTP URL to WebSocket URL
        let ws_url = self.convert_to_ws_url(&self.config.target)?;

        info!(
            "Starting WebSocket load test against {} with {} concurrent connections",
            ws_url, self.config.concurrency
        );

        // Warm-up phase
        if let Some(warmup) = self.config.warmup {
            info!("Starting warm-up phase for {:?}", warmup);
            self.run_warmup(&ws_url, warmup).await?;
            self.metrics.reset().await;
            info!("Warm-up complete, starting measurement");
        }

        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));
        let test_duration = self.config.duration;
        let test_requests = self.config.requests;
        let start_time = Instant::now();

        let mut handles = Vec::new();

        for worker_id in 0..self.config.concurrency {
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let running = Arc::clone(&self.running);
            let connection_count = Arc::clone(&self.connection_count);
            let semaphore = Arc::clone(&semaphore);
            let ws_url = ws_url.clone();

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

                    if let Some(max_conns) = test_requests {
                        let current = connection_count.fetch_add(1, Ordering::SeqCst);
                        if current >= max_conns {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }

                    let _permit = semaphore.acquire().await.unwrap();

                    let result = run_websocket_session(&ws_url, &config, &metrics, worker_id).await;

                    if let Err(e) = result {
                        debug!("WebSocket session error (worker {}): {:?}", worker_id, e);
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        self.running.store(false, Ordering::SeqCst);
        info!("WebSocket load test complete");

        Ok(())
    }

    async fn run_warmup(&self, ws_url: &str, duration: Duration) -> Result<(), WebSocketError> {
        let start = Instant::now();
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));

        while start.elapsed() < duration {
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let semaphore = Arc::clone(&semaphore);
            let ws_url = ws_url.to_string();

            let _permit = semaphore.acquire().await.unwrap();

            tokio::spawn(async move {
                let _ = run_websocket_session(&ws_url, &config, &metrics, 0).await;
            });
        }

        Ok(())
    }

    fn convert_to_ws_url(&self, url: &str) -> Result<String, WebSocketError> {
        let url = if url.starts_with("ws://") || url.starts_with("wss://") {
            url.to_string()
        } else if url.starts_with("https://") {
            url.replace("https://", "wss://")
        } else if url.starts_with("http://") {
            url.replace("http://", "ws://")
        } else {
            format!("ws://{}", url)
        };

        // Validate URL
        Url::parse(&url).map_err(|e| WebSocketError::InvalidUrl(e.to_string()))?;

        Ok(url)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Run a single WebSocket session
async fn run_websocket_session(
    ws_url: &str,
    config: &BenchConfig,
    metrics: &SharedMetrics,
    _worker_id: u32,
) -> Result<(), WebSocketError> {
    let ws_config = &config.websocket;

    // Connect
    let connect_start = Instant::now();
    let connect_result = tokio::time::timeout(config.timeout, connect_async(ws_url)).await;

    let (ws_stream, _response) = match connect_result {
        Ok(Ok((stream, response))) => {
            let connect_latency = connect_start.elapsed();
            metrics.latency.record(connect_latency).await;
            metrics.counters.record_success();
            (stream, response)
        }
        Ok(Err(e)) => {
            metrics.counters.record_connection_error();
            return Err(WebSocketError::Connection(e.to_string()));
        }
        Err(_) => {
            metrics.counters.record_timeout();
            return Err(WebSocketError::Timeout);
        }
    };

    let (mut write, mut read) = ws_stream.split();

    // Generate or use configured message
    let message = if let Some(ref msg) = ws_config.message {
        msg.clone()
    } else {
        generate_random_message(ws_config.message_size)
    };

    // Send messages
    for _ in 0..ws_config.messages_per_connection {
        let msg_start = Instant::now();

        // Send message
        let send_result = write.send(Message::Text(message.clone())).await;

        if let Err(e) = send_result {
            metrics.counters.record_error();
            debug!("WebSocket send error: {:?}", e);
            continue;
        }

        metrics.bytes.record_sent(message.len() as u64);

        // Wait for response (echo)
        let recv_result =
            tokio::time::timeout(Duration::from_secs(10), read.next()).await;

        match recv_result {
            Ok(Some(Ok(msg))) => {
                let latency = msg_start.elapsed();
                metrics.latency.record(latency).await;
                metrics.counters.record_success();

                match msg {
                    Message::Text(text) => {
                        metrics.bytes.record_received(text.len() as u64);
                    }
                    Message::Binary(data) => {
                        metrics.bytes.record_received(data.len() as u64);
                    }
                    Message::Close(_) => {
                        break;
                    }
                    _ => {}
                }
            }
            Ok(Some(Err(e))) => {
                metrics.counters.record_error();
                debug!("WebSocket receive error: {:?}", e);
            }
            Ok(None) => {
                // Connection closed
                break;
            }
            Err(_) => {
                metrics.counters.record_timeout();
            }
        }

        // Optional delay between messages
        if let Some(delay) = ws_config.message_delay {
            sleep(delay).await;
        }
    }

    // Close connection gracefully
    let _ = write.send(Message::Close(None)).await;

    Ok(())
}

fn generate_random_message(size: usize) -> String {
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = (0..size)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            match idx {
                0..=9 => (b'0' + idx) as char,
                10..=35 => (b'a' + idx - 10) as char,
                _ => (b'A' + idx - 36) as char,
            }
        })
        .collect();
    chars.into_iter().collect()
}

#[derive(Debug)]
pub enum WebSocketError {
    InvalidUrl(String),
    Connection(String),
    Timeout,
    Send(String),
    Receive(String),
}

impl std::fmt::Display for WebSocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebSocketError::InvalidUrl(e) => write!(f, "Invalid WebSocket URL: {}", e),
            WebSocketError::Connection(e) => write!(f, "WebSocket connection error: {}", e),
            WebSocketError::Timeout => write!(f, "WebSocket timeout"),
            WebSocketError::Send(e) => write!(f, "WebSocket send error: {}", e),
            WebSocketError::Receive(e) => write!(f, "WebSocket receive error: {}", e),
        }
    }
}

impl std::error::Error for WebSocketError {}

impl Clone for WebSocketTester {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            metrics: Arc::clone(&self.metrics),
            running: Arc::clone(&self.running),
            connection_count: Arc::clone(&self.connection_count),
        }
    }
}
