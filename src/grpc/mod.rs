use crate::config::BenchConfig;
use crate::metrics::SharedMetrics;
use bytes::{Buf, BufMut, Bytes};
use http::Uri;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};
use tracing::{debug, info};

/// gRPC load tester
///
/// Supports basic gRPC health check and custom unary calls
pub struct GrpcTester {
    config: BenchConfig,
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    request_count: Arc<AtomicU64>,
}

impl GrpcTester {
    pub fn new(config: BenchConfig, metrics: SharedMetrics) -> Self {
        Self {
            config,
            metrics,
            running: Arc::new(AtomicBool::new(false)),
            request_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Run the gRPC load test
    pub async fn run(&self) -> Result<(), GrpcError> {
        self.running.store(true, Ordering::SeqCst);
        self.request_count.store(0, Ordering::SeqCst);

        info!(
            "Starting gRPC load test against {} with {} concurrent connections",
            self.config.target, self.config.concurrency
        );

        // Parse target
        let target = self.normalize_grpc_url(&self.config.target)?;

        // Create channel
        let channel = self.create_channel(&target).await?;

        // Warm-up phase
        if let Some(warmup) = self.config.warmup {
            info!("Starting warm-up phase for {:?}", warmup);
            self.run_warmup(channel.clone(), warmup).await?;
            self.metrics.reset().await;
            info!("Warm-up complete, starting measurement");
        }

        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));
        let test_duration = self.config.duration;
        let test_requests = self.config.requests;
        let start_time = Instant::now();

        let mut handles = Vec::new();

        for _ in 0..self.config.concurrency {
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let running = Arc::clone(&self.running);
            let request_count = Arc::clone(&self.request_count);
            let semaphore = Arc::clone(&semaphore);
            let channel = channel.clone();

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

                    let _permit = semaphore.acquire().await.unwrap();

                    let result = execute_grpc_request(channel.clone(), &config, &metrics).await;

                    if let Err(e) = result {
                        debug!("gRPC request error: {:?}", e);
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        self.running.store(false, Ordering::SeqCst);
        info!("gRPC load test complete");

        Ok(())
    }

    async fn run_warmup(&self, channel: Channel, duration: Duration) -> Result<(), GrpcError> {
        let start = Instant::now();
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));

        while start.elapsed() < duration {
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let semaphore = Arc::clone(&semaphore);
            let channel = channel.clone();

            let _permit = semaphore.acquire().await.unwrap();

            tokio::spawn(async move {
                let _ = execute_grpc_request(channel, &config, &metrics).await;
            });
        }

        Ok(())
    }

    fn normalize_grpc_url(&self, url: &str) -> Result<String, GrpcError> {
        // Handle various URL formats
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(url.to_string())
        } else if url.starts_with("grpc://") {
            Ok(url.replace("grpc://", "http://"))
        } else if url.starts_with("grpcs://") {
            Ok(url.replace("grpcs://", "https://"))
        } else {
            // Assume plaintext if no scheme
            Ok(format!("http://{}", url))
        }
    }

    async fn create_channel(&self, target: &str) -> Result<Channel, GrpcError> {
        let uri: Uri = target
            .parse()
            .map_err(|e: http::uri::InvalidUri| GrpcError::InvalidTarget(e.to_string()))?;

        let endpoint = Endpoint::from(uri)
            .timeout(self.config.timeout)
            .connect_timeout(Duration::from_secs(10));

        let channel = if !self.config.grpc.plaintext && self.config.grpc.tls {
            endpoint
                .tls_config(tonic::transport::ClientTlsConfig::new())
                .map_err(|e| GrpcError::TlsConfig(e.to_string()))?
                .connect()
                .await
                .map_err(|e| GrpcError::Connection(e.to_string()))?
        } else {
            endpoint
                .connect()
                .await
                .map_err(|e| GrpcError::Connection(e.to_string()))?
        };

        Ok(channel)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Execute a single gRPC request
async fn execute_grpc_request(
    channel: Channel,
    config: &BenchConfig,
    metrics: &SharedMetrics,
) -> Result<(), GrpcError> {
    let grpc_config = &config.grpc;

    // Determine what type of request to make
    let service = grpc_config
        .service
        .as_deref()
        .unwrap_or("grpc.health.v1.Health");
    let method = grpc_config.method.as_deref().unwrap_or("Check");

    // Build path
    let path = format!("/{}/{}", service, method);

    let start = Instant::now();

    // Use raw gRPC call
    let result = make_raw_grpc_call(channel, &path, grpc_config.request.as_deref(), &grpc_config.metadata).await;

    let latency = start.elapsed();

    match result {
        Ok(response_bytes) => {
            metrics.latency.record(latency).await;
            metrics.counters.record_success();
            metrics.bytes.record_received(response_bytes.len() as u64);
            // Track gRPC OK status as HTTP 200
            metrics.status_codes.record(200).await;
            Ok(())
        }
        Err(status) => {
            metrics.latency.record(latency).await;

            // Map gRPC status to HTTP-like status codes for consistency
            let http_status = grpc_status_to_http(status.code());
            metrics.status_codes.record(http_status).await;

            if status.code() == tonic::Code::DeadlineExceeded {
                metrics.counters.record_timeout();
            } else if status.code() == tonic::Code::Unavailable {
                metrics.counters.record_connection_error();
            } else {
                metrics.counters.record_error();
            }

            Err(GrpcError::Status(status.to_string()))
        }
    }
}

/// Make a raw gRPC call using tonic's low-level API
async fn make_raw_grpc_call(
    channel: Channel,
    path: &str,
    request_body: Option<&str>,
    metadata: &std::collections::HashMap<String, String>,
) -> Result<Bytes, Status> {
    use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};

    // Create a simple pass-through codec for raw bytes
    #[derive(Default, Clone)]
    struct RawCodec;

    impl Codec for RawCodec {
        type Encode = Bytes;
        type Decode = Bytes;
        type Encoder = RawEncoder;
        type Decoder = RawDecoder;

        fn encoder(&mut self) -> Self::Encoder {
            RawEncoder
        }

        fn decoder(&mut self) -> Self::Decoder {
            RawDecoder
        }
    }

    struct RawEncoder;
    impl Encoder for RawEncoder {
        type Item = Bytes;
        type Error = Status;

        fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
            dst.reserve(item.len());
            dst.put_slice(&item);
            Ok(())
        }
    }

    struct RawDecoder;
    impl Decoder for RawDecoder {
        type Item = Bytes;
        type Error = Status;

        fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
            let data = src.copy_to_bytes(src.remaining());
            Ok(Some(data))
        }
    }

    // Build request
    let request_bytes = if let Some(body) = request_body {
        // Try to parse as JSON and convert to protobuf-like format
        // For now, just send as raw bytes
        Bytes::from(body.to_string())
    } else {
        // Empty request (like health check)
        Bytes::new()
    };

    let mut request = Request::new(request_bytes);

    // Add metadata
    for (key, value) in metadata {
        if let Ok(meta_key) = key.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(meta_val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                request.metadata_mut().insert(meta_key, meta_val);
            }
        }
    }

    // Make the call using the Grpc client
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready().await.map_err(|e| Status::unavailable(e.to_string()))?;

    let codec = RawCodec;
    let path_uri: http::uri::PathAndQuery = path.parse().map_err(|_| Status::invalid_argument("Invalid path"))?;

    let response = grpc
        .unary(request, path_uri, codec)
        .await?;

    Ok(response.into_inner())
}

/// Map gRPC status code to HTTP status for metrics consistency
fn grpc_status_to_http(code: tonic::Code) -> u16 {
    match code {
        tonic::Code::Ok => 200,
        tonic::Code::Cancelled => 499,
        tonic::Code::Unknown => 500,
        tonic::Code::InvalidArgument => 400,
        tonic::Code::DeadlineExceeded => 504,
        tonic::Code::NotFound => 404,
        tonic::Code::AlreadyExists => 409,
        tonic::Code::PermissionDenied => 403,
        tonic::Code::ResourceExhausted => 429,
        tonic::Code::FailedPrecondition => 400,
        tonic::Code::Aborted => 409,
        tonic::Code::OutOfRange => 400,
        tonic::Code::Unimplemented => 501,
        tonic::Code::Internal => 500,
        tonic::Code::Unavailable => 503,
        tonic::Code::DataLoss => 500,
        tonic::Code::Unauthenticated => 401,
    }
}

#[derive(Debug)]
pub enum GrpcError {
    InvalidTarget(String),
    Connection(String),
    TlsConfig(String),
    Status(String),
    Timeout,
}

impl std::fmt::Display for GrpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GrpcError::InvalidTarget(e) => write!(f, "Invalid gRPC target: {}", e),
            GrpcError::Connection(e) => write!(f, "gRPC connection error: {}", e),
            GrpcError::TlsConfig(e) => write!(f, "gRPC TLS configuration error: {}", e),
            GrpcError::Status(e) => write!(f, "gRPC status error: {}", e),
            GrpcError::Timeout => write!(f, "gRPC request timeout"),
        }
    }
}

impl std::error::Error for GrpcError {}

impl Clone for GrpcTester {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            metrics: Arc::clone(&self.metrics),
            running: Arc::clone(&self.running),
            request_count: Arc::clone(&self.request_count),
        }
    }
}

/// Simple gRPC health check client
pub mod health {
    use super::*;

    /// Perform a gRPC health check
    pub async fn check(channel: Channel, service: Option<&str>) -> Result<bool, GrpcError> {
        let path = "/grpc.health.v1.Health/Check";
        let request_body = if let Some(svc) = service {
            Some(format!(r#"{{"service":"{}"}}"#, svc))
        } else {
            None
        };

        let result = make_raw_grpc_call(
            channel,
            path,
            request_body.as_deref(),
            &std::collections::HashMap::new(),
        )
        .await;

        match result {
            Ok(_) => Ok(true),
            Err(status) => {
                if status.code() == tonic::Code::NotFound {
                    Ok(false) // Service not found but gRPC is working
                } else {
                    Err(GrpcError::Status(status.to_string()))
                }
            }
        }
    }
}

