use crate::config::BenchConfig;
use crate::metrics::SharedMetrics;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tracing::{debug, info};

/// TCP load tester
pub struct TcpTester {
    config: BenchConfig,
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    connection_count: Arc<AtomicU64>,
}

impl TcpTester {
    pub fn new(config: BenchConfig, metrics: SharedMetrics) -> Self {
        Self {
            config,
            metrics,
            running: Arc::new(AtomicBool::new(false)),
            connection_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Run the TCP load test
    pub async fn run(&self) -> Result<(), TcpError> {
        self.running.store(true, Ordering::SeqCst);
        self.connection_count.store(0, Ordering::SeqCst);

        let addr = self.parse_address(&self.config.target)?;

        info!(
            "Starting TCP load test against {} with {} concurrent connections",
            addr, self.config.concurrency
        );

        // Warm-up phase
        if let Some(warmup) = self.config.warmup {
            info!("Starting warm-up phase for {:?}", warmup);
            self.run_warmup(&addr, warmup).await?;
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
            let connection_count = Arc::clone(&self.connection_count);
            let semaphore = Arc::clone(&semaphore);
            let addr = addr.clone();

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

                    let result = run_tcp_session(&addr, &config, &metrics).await;

                    if let Err(e) = result {
                        debug!("TCP session error: {:?}", e);
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        self.running.store(false, Ordering::SeqCst);
        info!("TCP load test complete");

        Ok(())
    }

    async fn run_warmup(&self, addr: &str, duration: Duration) -> Result<(), TcpError> {
        let start = Instant::now();
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));

        while start.elapsed() < duration {
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            let semaphore = Arc::clone(&semaphore);
            let addr = addr.to_string();

            let _permit = semaphore.acquire().await.unwrap();

            tokio::spawn(async move {
                let _ = run_tcp_session(&addr, &config, &metrics).await;
            });
        }

        Ok(())
    }

    fn parse_address(&self, target: &str) -> Result<String, TcpError> {
        // Strip any protocol prefix
        let addr = target
            .trim_start_matches("tcp://")
            .trim_start_matches("tls://")
            .trim_start_matches("tcps://");

        // Validate it looks like host:port
        if !addr.contains(':') {
            return Err(TcpError::InvalidAddress(
                "Address must be in host:port format".to_string(),
            ));
        }

        Ok(addr.to_string())
    }

    #[allow(dead_code)]
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Run a single TCP session
async fn run_tcp_session(
    addr: &str,
    config: &BenchConfig,
    metrics: &SharedMetrics,
) -> Result<(), TcpError> {
    let tcp_config = &config.tcp;

    let connect_start = Instant::now();

    // Connect with timeout
    let connect_result = tokio::time::timeout(config.timeout, TcpStream::connect(addr)).await;

    let stream = match connect_result {
        Ok(Ok(stream)) => {
            metrics.latency.record(connect_start.elapsed()).await;
            stream
        }
        Ok(Err(e)) => {
            metrics.counters.record_connection_error();
            return Err(TcpError::Connection(e.to_string()));
        }
        Err(_) => {
            metrics.counters.record_timeout();
            return Err(TcpError::Timeout);
        }
    };

    // TLS if configured
    if tcp_config.tls {
        run_tls_session(stream, addr, config, metrics).await
    } else {
        run_plain_session(stream, config, metrics).await
    }
}

async fn run_plain_session(
    mut stream: TcpStream,
    config: &BenchConfig,
    metrics: &SharedMetrics,
) -> Result<(), TcpError> {
    let tcp_config = &config.tcp;

    // Get data to send
    let send_data = get_send_data(tcp_config)?;

    if let Some(data) = send_data {
        let start = Instant::now();

        // Send data
        stream
            .write_all(&data)
            .await
            .map_err(|e| TcpError::Write(e.to_string()))?;

        metrics.bytes.record_sent(data.len() as u64);

        // Read response
        let mut buf = vec![0u8; tcp_config.read_size];
        let read_result =
            tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf)).await;

        match read_result {
            Ok(Ok(n)) => {
                let latency = start.elapsed();
                metrics.latency.record(latency).await;
                metrics.bytes.record_received(n as u64);

                // Validate response if expected
                if let Some(ref expected) = tcp_config.expect {
                    let response = String::from_utf8_lossy(&buf[..n]);
                    if response.contains(expected) {
                        metrics.counters.record_success();
                    } else {
                        metrics.counters.record_error();
                    }
                } else {
                    metrics.counters.record_success();
                }
            }
            Ok(Err(e)) => {
                metrics.counters.record_error();
                return Err(TcpError::Read(e.to_string()));
            }
            Err(_) => {
                metrics.counters.record_timeout();
            }
        }
    } else {
        // Just connection test
        metrics.counters.record_success();
    }

    Ok(())
}

async fn run_tls_session(
    stream: TcpStream,
    addr: &str,
    config: &BenchConfig,
    metrics: &SharedMetrics,
) -> Result<(), TcpError> {
    let tcp_config = &config.tcp;

    // Extract hostname from address
    let hostname = addr
        .split(':')
        .next()
        .ok_or_else(|| TcpError::InvalidAddress("Cannot extract hostname".to_string()))?;

    // Build TLS config
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = if tcp_config.insecure {
        // Allow invalid certs for testing
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier))
            .with_no_client_auth()
    } else {
        ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };

    let connector = TlsConnector::from(Arc::new(tls_config));

    let server_name: tokio_rustls::rustls::pki_types::ServerName<'static> = hostname
        .to_string()
        .try_into()
        .map_err(|_| TcpError::InvalidAddress("Invalid server name".to_string()))?;

    let tls_start = Instant::now();
    let tls_result =
        tokio::time::timeout(config.timeout, connector.connect(server_name, stream)).await;

    let mut tls_stream = match tls_result {
        Ok(Ok(stream)) => {
            metrics.latency.record(tls_start.elapsed()).await;
            stream
        }
        Ok(Err(e)) => {
            metrics.counters.record_connection_error();
            return Err(TcpError::Tls(e.to_string()));
        }
        Err(_) => {
            metrics.counters.record_timeout();
            return Err(TcpError::Timeout);
        }
    };

    // Get data to send
    let send_data = get_send_data(tcp_config)?;

    if let Some(data) = send_data {
        let start = Instant::now();

        // Send data
        tls_stream
            .write_all(&data)
            .await
            .map_err(|e| TcpError::Write(e.to_string()))?;

        metrics.bytes.record_sent(data.len() as u64);

        // Read response
        let mut buf = vec![0u8; tcp_config.read_size];
        let read_result =
            tokio::time::timeout(Duration::from_secs(10), tls_stream.read(&mut buf)).await;

        match read_result {
            Ok(Ok(n)) => {
                let latency = start.elapsed();
                metrics.latency.record(latency).await;
                metrics.bytes.record_received(n as u64);

                if let Some(ref expected) = tcp_config.expect {
                    let response = String::from_utf8_lossy(&buf[..n]);
                    if response.contains(expected) {
                        metrics.counters.record_success();
                    } else {
                        metrics.counters.record_error();
                    }
                } else {
                    metrics.counters.record_success();
                }
            }
            Ok(Err(e)) => {
                metrics.counters.record_error();
                return Err(TcpError::Read(e.to_string()));
            }
            Err(_) => {
                metrics.counters.record_timeout();
            }
        }
    } else {
        metrics.counters.record_success();
    }

    Ok(())
}

fn get_send_data(tcp_config: &crate::config::TcpConfig) -> Result<Option<Vec<u8>>, TcpError> {
    if let Some(ref data) = tcp_config.send_data {
        Ok(Some(data.as_bytes().to_vec()))
    } else if let Some(ref hex) = tcp_config.send_hex {
        let bytes = hex::decode(hex).map_err(|e| TcpError::InvalidData(e.to_string()))?;
        Ok(Some(bytes))
    } else {
        Ok(None)
    }
}

/// Insecure certificate verifier for testing
#[derive(Debug)]
struct InsecureVerifier;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        vec![
            tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA256,
            tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA384,
            tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA512,
            tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            tokio_rustls::rustls::SignatureScheme::RSA_PSS_SHA256,
            tokio_rustls::rustls::SignatureScheme::RSA_PSS_SHA384,
            tokio_rustls::rustls::SignatureScheme::RSA_PSS_SHA512,
            tokio_rustls::rustls::SignatureScheme::ED25519,
        ]
    }
}

#[derive(Debug)]
pub enum TcpError {
    InvalidAddress(String),
    Connection(String),
    Tls(String),
    Write(String),
    Read(String),
    InvalidData(String),
    Timeout,
}

impl std::fmt::Display for TcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TcpError::InvalidAddress(e) => write!(f, "Invalid TCP address: {}", e),
            TcpError::Connection(e) => write!(f, "TCP connection error: {}", e),
            TcpError::Tls(e) => write!(f, "TLS error: {}", e),
            TcpError::Write(e) => write!(f, "TCP write error: {}", e),
            TcpError::Read(e) => write!(f, "TCP read error: {}", e),
            TcpError::InvalidData(e) => write!(f, "Invalid data: {}", e),
            TcpError::Timeout => write!(f, "TCP timeout"),
        }
    }
}

impl std::error::Error for TcpError {}

impl Clone for TcpTester {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            metrics: Arc::clone(&self.metrics),
            running: Arc::clone(&self.running),
            connection_count: Arc::clone(&self.connection_count),
        }
    }
}
