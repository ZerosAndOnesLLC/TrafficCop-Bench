use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Main configuration for load tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchConfig {
    /// Test name for identification
    #[serde(default = "default_test_name")]
    pub name: String,

    /// Target URL or address
    pub target: String,

    /// Test duration (e.g., "30s", "5m", "1h")
    #[serde(default, with = "option_duration_serde")]
    pub duration: Option<Duration>,

    /// Total number of requests to send (alternative to duration)
    #[serde(default)]
    pub requests: Option<u64>,

    /// Number of concurrent connections/workers
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,

    /// Target requests per second (0 = unlimited)
    #[serde(default)]
    pub rate: u64,

    /// Warm-up duration before collecting metrics
    #[serde(default, with = "option_duration_serde")]
    pub warmup: Option<Duration>,

    /// Request timeout
    #[serde(default = "default_timeout", with = "duration_serde")]
    pub timeout: Duration,

    /// HTTP-specific configuration
    #[serde(default)]
    pub http: HttpConfig,

    /// WebSocket-specific configuration
    #[serde(default)]
    pub websocket: WebSocketConfig,

    /// gRPC-specific configuration
    #[serde(default)]
    pub grpc: GrpcConfig,

    /// TCP-specific configuration
    #[serde(default)]
    pub tcp: TcpConfig,

    /// Output configuration
    #[serde(default)]
    pub output: OutputConfig,
}

fn default_test_name() -> String {
    "load-test".to_string()
}

fn default_concurrency() -> u32 {
    10
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

/// HTTP test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// HTTP method
    #[serde(default = "default_http_method")]
    pub method: String,

    /// Request headers
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Request body (for POST/PUT/PATCH)
    #[serde(default)]
    pub body: Option<String>,

    /// Body file path (alternative to inline body)
    #[serde(default)]
    pub body_file: Option<String>,

    /// Content-Type header
    #[serde(default)]
    pub content_type: Option<String>,

    /// Keep-alive connections
    #[serde(default = "default_true")]
    pub keep_alive: bool,

    /// Enable HTTP/2
    #[serde(default = "default_true")]
    pub http2: bool,

    /// Skip TLS verification (for self-signed certs)
    #[serde(default)]
    pub insecure: bool,

    /// Multiple endpoints to test (round-robin or random)
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,

    /// Expected status codes (default: 200-299)
    #[serde(default = "default_expected_status")]
    pub expected_status: Vec<u16>,
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_true() -> bool {
    true
}

fn default_expected_status() -> Vec<u16> {
    (200..300).collect()
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            method: default_http_method(),
            headers: HashMap::new(),
            body: None,
            body_file: None,
            content_type: None,
            keep_alive: true,
            http2: true,
            insecure: false,
            endpoints: Vec::new(),
            expected_status: default_expected_status(),
        }
    }
}

/// Individual endpoint configuration for multi-endpoint tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    /// Endpoint path or full URL
    pub path: String,

    /// HTTP method (overrides global)
    #[serde(default)]
    pub method: Option<String>,

    /// Request body
    #[serde(default)]
    pub body: Option<String>,

    /// Weight for weighted distribution
    #[serde(default = "default_weight")]
    pub weight: u32,

    /// Additional headers for this endpoint
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_weight() -> u32 {
    1
}

/// WebSocket test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// Messages to send per connection
    #[serde(default = "default_messages_per_conn")]
    pub messages_per_connection: u32,

    /// Message payload
    #[serde(default)]
    pub message: Option<String>,

    /// Message size in bytes (generates random data if message is not set)
    #[serde(default = "default_message_size")]
    pub message_size: usize,

    /// Delay between messages
    #[serde(default, with = "option_duration_serde")]
    pub message_delay: Option<Duration>,

    /// Subprotocols to request
    #[serde(default)]
    pub protocols: Vec<String>,

    /// Additional headers
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_messages_per_conn() -> u32 {
    10
}

fn default_message_size() -> usize {
    256
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            messages_per_connection: default_messages_per_conn(),
            message: None,
            message_size: default_message_size(),
            message_delay: None,
            protocols: Vec::new(),
            headers: HashMap::new(),
        }
    }
}

/// gRPC test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcConfig {
    /// Service name (e.g., "grpc.health.v1.Health")
    #[serde(default)]
    pub service: Option<String>,

    /// Method name (e.g., "Check")
    #[serde(default)]
    pub method: Option<String>,

    /// Proto file path for reflection
    #[serde(default)]
    pub proto_file: Option<String>,

    /// Request payload as JSON
    #[serde(default)]
    pub request: Option<String>,

    /// Metadata headers
    #[serde(default)]
    pub metadata: HashMap<String, String>,

    /// Enable TLS
    #[serde(default)]
    pub tls: bool,

    /// Use plaintext (no TLS)
    #[serde(default = "default_true")]
    pub plaintext: bool,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            service: None,
            method: None,
            proto_file: None,
            request: None,
            metadata: HashMap::new(),
            tls: false,
            plaintext: true,
        }
    }
}

/// TCP test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpConfig {
    /// Data to send
    #[serde(default)]
    pub send_data: Option<String>,

    /// Hex-encoded data to send
    #[serde(default)]
    pub send_hex: Option<String>,

    /// Expected response (for validation)
    #[serde(default)]
    pub expect: Option<String>,

    /// Bytes to read from response
    #[serde(default = "default_tcp_read_size")]
    pub read_size: usize,

    /// Enable TLS
    #[serde(default)]
    pub tls: bool,

    /// Skip TLS verification
    #[serde(default)]
    pub insecure: bool,
}

fn default_tcp_read_size() -> usize {
    4096
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            send_data: None,
            send_hex: None,
            expect: None,
            read_size: default_tcp_read_size(),
            tls: false,
            insecure: false,
        }
    }
}

/// Output configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Output format: json, csv, text
    #[serde(default = "default_output_format")]
    pub format: String,

    /// Output file path (stdout if not set)
    #[serde(default)]
    pub file: Option<String>,

    /// Show real-time progress
    #[serde(default = "default_true")]
    pub progress: bool,

    /// Progress update interval
    #[serde(default = "default_progress_interval", with = "duration_serde")]
    pub progress_interval: Duration,

    /// Export detailed histogram data
    #[serde(default)]
    pub histogram_file: Option<String>,
}

fn default_output_format() -> String {
    "text".to_string()
}

fn default_progress_interval() -> Duration {
    Duration::from_secs(1)
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: default_output_format(),
            file: None,
            progress: true,
            progress_interval: default_progress_interval(),
            histogram_file: None,
        }
    }
}

impl BenchConfig {
    /// Load configuration from YAML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::IoError(e.to_string()))?;
        serde_yaml::from_str(&content).map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Create a simple HTTP GET test configuration
    pub fn http_get(url: &str, concurrency: u32, duration: Duration) -> Self {
        Self {
            name: "http-get-test".to_string(),
            target: url.to_string(),
            duration: Some(duration),
            requests: None,
            concurrency,
            rate: 0,
            warmup: None,
            timeout: default_timeout(),
            http: HttpConfig::default(),
            websocket: WebSocketConfig::default(),
            grpc: GrpcConfig::default(),
            tcp: TcpConfig::default(),
            output: OutputConfig::default(),
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.target.is_empty() {
            return Err(ConfigError::ValidationError(
                "Target URL/address is required".to_string(),
            ));
        }

        if self.duration.is_none() && self.requests.is_none() {
            return Err(ConfigError::ValidationError(
                "Either duration or requests count is required".to_string(),
            ));
        }

        if self.concurrency == 0 {
            return Err(ConfigError::ValidationError(
                "Concurrency must be at least 1".to_string(),
            ));
        }

        Ok(())
    }
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            name: default_test_name(),
            target: String::new(),
            duration: Some(Duration::from_secs(30)),
            requests: None,
            concurrency: default_concurrency(),
            rate: 0,
            warmup: None,
            timeout: default_timeout(),
            http: HttpConfig::default(),
            websocket: WebSocketConfig::default(),
            grpc: GrpcConfig::default(),
            tcp: TcpConfig::default(),
            output: OutputConfig::default(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    IoError(String),
    ParseError(String),
    ValidationError(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::IoError(e) => write!(f, "IO error: {}", e),
            ConfigError::ParseError(e) => write!(f, "Parse error: {}", e),
            ConfigError::ValidationError(e) => write!(f, "Validation error: {}", e),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Duration serialization/deserialization for required Duration fields
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format_duration(*duration);
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_duration(&s).map_err(serde::de::Error::custom)
    }

    fn format_duration(d: Duration) -> String {
        let secs = d.as_secs();
        if secs >= 3600 {
            format!("{}h", secs / 3600)
        } else if secs >= 60 {
            format!("{}m", secs / 60)
        } else {
            format!("{}s", secs)
        }
    }

    pub fn parse_duration(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("Empty duration string".to_string());
        }

        let (num_str, unit) = if s.ends_with("ms") {
            (&s[..s.len() - 2], "ms")
        } else if s.ends_with('s') {
            (&s[..s.len() - 1], "s")
        } else if s.ends_with('m') {
            (&s[..s.len() - 1], "m")
        } else if s.ends_with('h') {
            (&s[..s.len() - 1], "h")
        } else {
            return Err(format!("Invalid duration format: {}", s));
        };

        let num: u64 = num_str
            .trim()
            .parse()
            .map_err(|_| format!("Invalid number in duration: {}", num_str))?;

        let duration = match unit {
            "ms" => Duration::from_millis(num),
            "s" => Duration::from_secs(num),
            "m" => Duration::from_secs(num * 60),
            "h" => Duration::from_secs(num * 3600),
            _ => return Err(format!("Unknown duration unit: {}", unit)),
        };

        Ok(duration)
    }
}

/// Duration serialization/deserialization for Option<Duration> fields
mod option_duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => {
                let s = format_duration(*d);
                serializer.serialize_some(&s)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let d = super::duration_serde::parse_duration(&s)
                    .map_err(serde::de::Error::custom)?;
                Ok(Some(d))
            }
            None => Ok(None),
        }
    }

    fn format_duration(d: Duration) -> String {
        let secs = d.as_secs();
        if secs >= 3600 {
            format!("{}h", secs / 3600)
        } else if secs >= 60 {
            format!("{}m", secs / 60)
        } else {
            format!("{}s", secs)
        }
    }
}

pub fn parse_duration(s: &str) -> Result<Duration, String> {
    duration_serde::parse_duration(s)
}
