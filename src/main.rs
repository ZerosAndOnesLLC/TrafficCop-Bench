mod config;
mod display;
mod grpc;
mod http;
mod metrics;
mod report;
mod tcp;
mod websocket;

use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use config::{parse_duration, BenchConfig};
use display::{SimpleProgress, StatsDisplay};
use metrics::new_shared_metrics;
use report::{OutputFormat, TestReport};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(
    name = "trafficcop-bench",
    about = "High-performance load testing tool for TrafficCop reverse proxy",
    version,
    author
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run HTTP load test
    Http(HttpArgs),

    /// Run WebSocket load test
    Ws(WebSocketArgs),

    /// Run gRPC load test
    Grpc(GrpcArgs),

    /// Run TCP load test
    Tcp(TcpArgs),

    /// Run load test from configuration file
    Run(RunArgs),

    /// Compare two test result files
    Compare(CompareArgs),
}

#[derive(Parser)]
struct HttpArgs {
    /// Target URL
    #[arg(required = true)]
    url: String,

    /// Number of concurrent connections
    #[arg(short, long, default_value = "10")]
    concurrency: u32,

    /// Test duration (e.g., "30s", "5m", "1h")
    #[arg(short, long, default_value = "30s")]
    duration: String,

    /// Number of requests to send (alternative to duration)
    #[arg(short = 'n', long)]
    requests: Option<u64>,

    /// Target requests per second (0 = unlimited)
    #[arg(short, long, default_value = "0")]
    rate: u64,

    /// HTTP method
    #[arg(short, long, default_value = "GET")]
    method: String,

    /// Request body
    #[arg(short, long)]
    body: Option<String>,

    /// Request headers (can be specified multiple times)
    #[arg(short = 'H', long = "header", value_parser = parse_header)]
    headers: Vec<(String, String)>,

    /// Request timeout
    #[arg(long, default_value = "30s")]
    timeout: String,

    /// Warm-up duration before collecting metrics
    #[arg(long)]
    warmup: Option<String>,

    /// Skip TLS certificate verification
    #[arg(long)]
    insecure: bool,

    /// Disable HTTP/2
    #[arg(long)]
    no_http2: bool,

    /// Output format
    #[arg(short, long, default_value = "text")]
    output: OutputFormatArg,

    /// Output file path
    #[arg(long)]
    output_file: Option<String>,

    /// Disable real-time progress display
    #[arg(long)]
    no_progress: bool,

    /// Use simple progress output instead of TUI
    #[arg(long)]
    simple_progress: bool,

    /// Test name for reports
    #[arg(long, default_value = "http-load-test")]
    name: String,
}

#[derive(Parser)]
struct WebSocketArgs {
    /// Target URL (ws:// or wss://)
    #[arg(required = true)]
    url: String,

    /// Number of concurrent connections
    #[arg(short, long, default_value = "10")]
    concurrency: u32,

    /// Test duration (e.g., "30s", "5m", "1h")
    #[arg(short, long, default_value = "30s")]
    duration: String,

    /// Number of connections to make (alternative to duration)
    #[arg(short = 'n', long)]
    connections: Option<u64>,

    /// Messages to send per connection
    #[arg(long, default_value = "10")]
    messages: u32,

    /// Message payload
    #[arg(short, long)]
    message: Option<String>,

    /// Message size in bytes (for random data)
    #[arg(long, default_value = "256")]
    message_size: usize,

    /// Connection timeout
    #[arg(long, default_value = "30s")]
    timeout: String,

    /// Warm-up duration
    #[arg(long)]
    warmup: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "text")]
    output: OutputFormatArg,

    /// Output file path
    #[arg(long)]
    output_file: Option<String>,

    /// Disable real-time progress display
    #[arg(long)]
    no_progress: bool,

    /// Use simple progress output instead of TUI
    #[arg(long)]
    simple_progress: bool,

    /// Test name for reports
    #[arg(long, default_value = "websocket-load-test")]
    name: String,
}

#[derive(Parser)]
struct GrpcArgs {
    /// Target address (host:port)
    #[arg(required = true)]
    target: String,

    /// Number of concurrent connections
    #[arg(short, long, default_value = "10")]
    concurrency: u32,

    /// Test duration (e.g., "30s", "5m", "1h")
    #[arg(short, long, default_value = "30s")]
    duration: String,

    /// Number of requests to send (alternative to duration)
    #[arg(short = 'n', long)]
    requests: Option<u64>,

    /// Target requests per second (0 = unlimited)
    #[arg(short, long, default_value = "0")]
    rate: u64,

    /// gRPC service name (e.g., "grpc.health.v1.Health")
    #[arg(long)]
    service: Option<String>,

    /// gRPC method name (e.g., "Check")
    #[arg(long)]
    method: Option<String>,

    /// Request payload as JSON
    #[arg(long)]
    request: Option<String>,

    /// Connection timeout
    #[arg(long, default_value = "30s")]
    timeout: String,

    /// Use TLS
    #[arg(long)]
    tls: bool,

    /// Warm-up duration
    #[arg(long)]
    warmup: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "text")]
    output: OutputFormatArg,

    /// Output file path
    #[arg(long)]
    output_file: Option<String>,

    /// Disable real-time progress display
    #[arg(long)]
    no_progress: bool,

    /// Use simple progress output instead of TUI
    #[arg(long)]
    simple_progress: bool,

    /// Test name for reports
    #[arg(long, default_value = "grpc-load-test")]
    name: String,
}

#[derive(Parser)]
struct TcpArgs {
    /// Target address (host:port)
    #[arg(required = true)]
    target: String,

    /// Number of concurrent connections
    #[arg(short, long, default_value = "10")]
    concurrency: u32,

    /// Test duration (e.g., "30s", "5m", "1h")
    #[arg(short, long, default_value = "30s")]
    duration: String,

    /// Number of connections to make (alternative to duration)
    #[arg(short = 'n', long)]
    connections: Option<u64>,

    /// Data to send
    #[arg(long)]
    send: Option<String>,

    /// Hex-encoded data to send
    #[arg(long)]
    send_hex: Option<String>,

    /// Expected response (for validation)
    #[arg(long)]
    expect: Option<String>,

    /// Connection timeout
    #[arg(long, default_value = "30s")]
    timeout: String,

    /// Use TLS
    #[arg(long)]
    tls: bool,

    /// Skip TLS certificate verification
    #[arg(long)]
    insecure: bool,

    /// Warm-up duration
    #[arg(long)]
    warmup: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "text")]
    output: OutputFormatArg,

    /// Output file path
    #[arg(long)]
    output_file: Option<String>,

    /// Disable real-time progress display
    #[arg(long)]
    no_progress: bool,

    /// Use simple progress output instead of TUI
    #[arg(long)]
    simple_progress: bool,

    /// Test name for reports
    #[arg(long, default_value = "tcp-load-test")]
    name: String,
}

#[derive(Parser)]
struct RunArgs {
    /// Configuration file path
    #[arg(required = true)]
    config: String,

    /// Override output format
    #[arg(short, long)]
    output: Option<OutputFormatArg>,

    /// Override output file path
    #[arg(long)]
    output_file: Option<String>,

    /// Disable real-time progress display
    #[arg(long)]
    no_progress: bool,

    /// Use simple progress output instead of TUI
    #[arg(long)]
    simple_progress: bool,
}

#[derive(Parser)]
struct CompareArgs {
    /// Baseline result file (JSON)
    #[arg(required = true)]
    baseline: String,

    /// Current result file (JSON)
    #[arg(required = true)]
    current: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormatArg {
    Text,
    Json,
    Csv,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(arg: OutputFormatArg) -> Self {
        match arg {
            OutputFormatArg::Text => OutputFormat::Text,
            OutputFormatArg::Json => OutputFormat::Json,
            OutputFormatArg::Csv => OutputFormat::Csv,
        }
    }
}

fn parse_header(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid header format: {}. Use 'Key: Value'", s));
    }
    Ok((parts[0].trim().to_string(), parts[1].trim().to_string()))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("trafficcop_bench=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Http(args) => run_http_test(args).await,
        Commands::Ws(args) => run_websocket_test(args).await,
        Commands::Grpc(args) => run_grpc_test(args).await,
        Commands::Tcp(args) => run_tcp_test(args).await,
        Commands::Run(args) => run_from_config(args).await,
        Commands::Compare(args) => compare_results(args),
    }
}

async fn run_http_test(args: HttpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let duration = parse_duration(&args.duration)?;
    let timeout = parse_duration(&args.timeout)?;
    let warmup = args.warmup.as_ref().map(|w| parse_duration(w)).transpose()?;

    let mut config = BenchConfig {
        name: args.name.clone(),
        target: args.url.clone(),
        duration: if args.requests.is_none() {
            Some(duration)
        } else {
            None
        },
        requests: args.requests,
        concurrency: args.concurrency,
        rate: args.rate,
        warmup,
        timeout,
        ..Default::default()
    };

    config.http.method = args.method;
    config.http.body = args.body;
    config.http.insecure = args.insecure;
    config.http.http2 = !args.no_http2;

    for (key, value) in args.headers {
        config.http.headers.insert(key, value);
    }

    let metrics = new_shared_metrics(&args.name);
    let started_at = Utc::now();

    // Create tester
    let tester = http::HttpTester::new(config.clone(), Arc::clone(&metrics))?;

    // Run test with progress display
    if !args.no_progress {
        if args.simple_progress {
            let progress = SimpleProgress::new(Arc::clone(&metrics), Duration::from_secs(1));
            let progress_flag = progress.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                async move { tester.run().await }
            });

            let progress_handle = tokio::spawn(async move { progress.run().await });

            let result = test_handle.await?;
            progress_flag.store(false, Ordering::SeqCst);
            let _ = progress_handle.await;

            result?;
        } else {
            let mut display = StatsDisplay::new(
                Arc::clone(&metrics),
                &args.name,
                &args.url,
                args.concurrency,
                config.duration,
            )?;
            let display_flag = display.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                let flag = Arc::clone(&display_flag);
                async move {
                    let result = tester.run().await;
                    flag.store(false, Ordering::SeqCst);
                    result
                }
            });

            display.run(Duration::from_millis(100)).await?;
            test_handle.await??;
        }
    } else {
        tester.run().await?;
    }

    // Generate report
    let snapshot = metrics.full_snapshot().await;
    let report = TestReport::from_snapshot(
        snapshot,
        &args.url,
        "HTTP",
        args.concurrency,
        if args.rate > 0 { Some(args.rate) } else { None },
        started_at,
    );

    // Output report
    output_report(&report, args.output.into(), args.output_file.as_deref())?;

    Ok(())
}

async fn run_websocket_test(args: WebSocketArgs) -> Result<(), Box<dyn std::error::Error>> {
    let duration = parse_duration(&args.duration)?;
    let timeout = parse_duration(&args.timeout)?;
    let warmup = args.warmup.as_ref().map(|w| parse_duration(w)).transpose()?;

    let mut config = BenchConfig {
        name: args.name.clone(),
        target: args.url.clone(),
        duration: if args.connections.is_none() {
            Some(duration)
        } else {
            None
        },
        requests: args.connections,
        concurrency: args.concurrency,
        warmup,
        timeout,
        ..Default::default()
    };

    config.websocket.messages_per_connection = args.messages;
    config.websocket.message = args.message;
    config.websocket.message_size = args.message_size;

    let metrics = new_shared_metrics(&args.name);
    let started_at = Utc::now();

    let tester = websocket::WebSocketTester::new(config.clone(), Arc::clone(&metrics));

    // Run test with progress display
    if !args.no_progress {
        if args.simple_progress {
            let progress = SimpleProgress::new(Arc::clone(&metrics), Duration::from_secs(1));
            let progress_flag = progress.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                async move { tester.run().await }
            });

            let progress_handle = tokio::spawn(async move { progress.run().await });

            let result = test_handle.await?;
            progress_flag.store(false, Ordering::SeqCst);
            let _ = progress_handle.await;

            result?;
        } else {
            let mut display = StatsDisplay::new(
                Arc::clone(&metrics),
                &args.name,
                &args.url,
                args.concurrency,
                config.duration,
            )?;
            let display_flag = display.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                let flag = Arc::clone(&display_flag);
                async move {
                    let result = tester.run().await;
                    flag.store(false, Ordering::SeqCst);
                    result
                }
            });

            display.run(Duration::from_millis(100)).await?;
            test_handle.await??;
        }
    } else {
        tester.run().await?;
    }

    // Generate report
    let snapshot = metrics.full_snapshot().await;
    let report = TestReport::from_snapshot(
        snapshot,
        &args.url,
        "WebSocket",
        args.concurrency,
        None,
        started_at,
    );

    output_report(&report, args.output.into(), args.output_file.as_deref())?;

    Ok(())
}

async fn run_grpc_test(args: GrpcArgs) -> Result<(), Box<dyn std::error::Error>> {
    let duration = parse_duration(&args.duration)?;
    let timeout = parse_duration(&args.timeout)?;
    let warmup = args.warmup.as_ref().map(|w| parse_duration(w)).transpose()?;

    let mut config = BenchConfig {
        name: args.name.clone(),
        target: args.target.clone(),
        duration: if args.requests.is_none() {
            Some(duration)
        } else {
            None
        },
        requests: args.requests,
        concurrency: args.concurrency,
        rate: args.rate,
        warmup,
        timeout,
        ..Default::default()
    };

    config.grpc.service = args.service;
    config.grpc.method = args.method;
    config.grpc.request = args.request;
    config.grpc.tls = args.tls;
    config.grpc.plaintext = !args.tls;

    let metrics = new_shared_metrics(&args.name);
    let started_at = Utc::now();

    let tester = grpc::GrpcTester::new(config.clone(), Arc::clone(&metrics));

    // Run test with progress display
    if !args.no_progress {
        if args.simple_progress {
            let progress = SimpleProgress::new(Arc::clone(&metrics), Duration::from_secs(1));
            let progress_flag = progress.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                async move { tester.run().await }
            });

            let progress_handle = tokio::spawn(async move { progress.run().await });

            let result = test_handle.await?;
            progress_flag.store(false, Ordering::SeqCst);
            let _ = progress_handle.await;

            result?;
        } else {
            let mut display = StatsDisplay::new(
                Arc::clone(&metrics),
                &args.name,
                &args.target,
                args.concurrency,
                config.duration,
            )?;
            let display_flag = display.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                let flag = Arc::clone(&display_flag);
                async move {
                    let result = tester.run().await;
                    flag.store(false, Ordering::SeqCst);
                    result
                }
            });

            display.run(Duration::from_millis(100)).await?;
            test_handle.await??;
        }
    } else {
        tester.run().await?;
    }

    // Generate report
    let snapshot = metrics.full_snapshot().await;
    let report = TestReport::from_snapshot(
        snapshot,
        &args.target,
        "gRPC",
        args.concurrency,
        if args.rate > 0 { Some(args.rate) } else { None },
        started_at,
    );

    output_report(&report, args.output.into(), args.output_file.as_deref())?;

    Ok(())
}

async fn run_tcp_test(args: TcpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let duration = parse_duration(&args.duration)?;
    let timeout = parse_duration(&args.timeout)?;
    let warmup = args.warmup.as_ref().map(|w| parse_duration(w)).transpose()?;

    let mut config = BenchConfig {
        name: args.name.clone(),
        target: args.target.clone(),
        duration: if args.connections.is_none() {
            Some(duration)
        } else {
            None
        },
        requests: args.connections,
        concurrency: args.concurrency,
        warmup,
        timeout,
        ..Default::default()
    };

    config.tcp.send_data = args.send;
    config.tcp.send_hex = args.send_hex;
    config.tcp.expect = args.expect;
    config.tcp.tls = args.tls;
    config.tcp.insecure = args.insecure;

    let metrics = new_shared_metrics(&args.name);
    let started_at = Utc::now();

    let tester = tcp::TcpTester::new(config.clone(), Arc::clone(&metrics));

    // Run test with progress display
    if !args.no_progress {
        if args.simple_progress {
            let progress = SimpleProgress::new(Arc::clone(&metrics), Duration::from_secs(1));
            let progress_flag = progress.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                async move { tester.run().await }
            });

            let progress_handle = tokio::spawn(async move { progress.run().await });

            let result = test_handle.await?;
            progress_flag.store(false, Ordering::SeqCst);
            let _ = progress_handle.await;

            result?;
        } else {
            let mut display = StatsDisplay::new(
                Arc::clone(&metrics),
                &args.name,
                &args.target,
                args.concurrency,
                config.duration,
            )?;
            let display_flag = display.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                let flag = Arc::clone(&display_flag);
                async move {
                    let result = tester.run().await;
                    flag.store(false, Ordering::SeqCst);
                    result
                }
            });

            display.run(Duration::from_millis(100)).await?;
            test_handle.await??;
        }
    } else {
        tester.run().await?;
    }

    // Generate report
    let snapshot = metrics.full_snapshot().await;
    let report = TestReport::from_snapshot(
        snapshot,
        &args.target,
        "TCP",
        args.concurrency,
        None,
        started_at,
    );

    output_report(&report, args.output.into(), args.output_file.as_deref())?;

    Ok(())
}

async fn run_from_config(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config = BenchConfig::from_file(&args.config)?;
    config.validate()?;

    info!("Loading configuration from {}", args.config);

    let metrics = new_shared_metrics(&config.name);
    let started_at = Utc::now();

    // Determine protocol from target URL
    let target = &config.target;
    let protocol = if target.starts_with("ws://") || target.starts_with("wss://") {
        "WebSocket"
    } else if target.starts_with("grpc://") || target.starts_with("grpcs://") {
        "gRPC"
    } else if target.starts_with("tcp://") || target.starts_with("tls://") {
        "TCP"
    } else {
        "HTTP"
    };

    // Run appropriate tester
    match protocol {
        "HTTP" => {
            let tester = http::HttpTester::new(config.clone(), Arc::clone(&metrics))?;
            run_with_progress(
                tester,
                &metrics,
                &config,
                args.no_progress,
                args.simple_progress,
            )
            .await?;
        }
        "WebSocket" => {
            let tester = websocket::WebSocketTester::new(config.clone(), Arc::clone(&metrics));
            run_with_progress(
                tester,
                &metrics,
                &config,
                args.no_progress,
                args.simple_progress,
            )
            .await?;
        }
        "gRPC" => {
            let tester = grpc::GrpcTester::new(config.clone(), Arc::clone(&metrics));
            run_with_progress(
                tester,
                &metrics,
                &config,
                args.no_progress,
                args.simple_progress,
            )
            .await?;
        }
        "TCP" => {
            let tester = tcp::TcpTester::new(config.clone(), Arc::clone(&metrics));
            run_with_progress(
                tester,
                &metrics,
                &config,
                args.no_progress,
                args.simple_progress,
            )
            .await?;
        }
        _ => unreachable!(),
    }

    // Generate report
    let snapshot = metrics.full_snapshot().await;
    let report = TestReport::from_snapshot(
        snapshot,
        &config.target,
        protocol,
        config.concurrency,
        if config.rate > 0 {
            Some(config.rate)
        } else {
            None
        },
        started_at,
    );

    let output_format = args.output.map(|o| o.into()).unwrap_or(OutputFormat::Text);
    output_report(&report, output_format, args.output_file.as_deref())?;

    Ok(())
}

async fn run_with_progress<T>(
    tester: T,
    metrics: &metrics::SharedMetrics,
    config: &BenchConfig,
    no_progress: bool,
    simple_progress: bool,
) -> Result<(), Box<dyn std::error::Error>>
where
    T: Tester + Clone + Send + 'static,
{
    if !no_progress && config.output.progress {
        if simple_progress {
            let progress =
                SimpleProgress::new(Arc::clone(metrics), config.output.progress_interval);
            let progress_flag = progress.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                async move { tester.run().await }
            });

            let progress_handle = tokio::spawn(async move { progress.run().await });

            match test_handle.await {
                Ok(result) => result.map_err(|e| -> Box<dyn std::error::Error> { e })?,
                Err(e) => return Err(Box::new(e)),
            }
            progress_flag.store(false, Ordering::SeqCst);
            let _ = progress_handle.await;
        } else {
            let mut display = StatsDisplay::new(
                Arc::clone(metrics),
                &config.name,
                &config.target,
                config.concurrency,
                config.duration,
            )?;
            let display_flag = display.running_flag();

            let test_handle = tokio::spawn({
                let tester = tester.clone();
                let flag = Arc::clone(&display_flag);
                async move {
                    let result = tester.run().await;
                    flag.store(false, Ordering::SeqCst);
                    result
                }
            });

            display.run(Duration::from_millis(100)).await?;
            match test_handle.await {
                Ok(result) => result.map_err(|e| -> Box<dyn std::error::Error> { e })?,
                Err(e) => return Err(Box::new(e)),
            }
        }
    } else {
        tester.run().await.map_err(|e| -> Box<dyn std::error::Error> { e })?;
    }

    Ok(())
}

fn compare_results(args: CompareArgs) -> Result<(), Box<dyn std::error::Error>> {
    let baseline_json = std::fs::read_to_string(&args.baseline)?;
    let current_json = std::fs::read_to_string(&args.current)?;

    let baseline: TestReport = serde_json::from_str(&baseline_json)?;
    let current: TestReport = serde_json::from_str(&current_json)?;

    let comparison = report::compare_reports(&baseline, &current);
    comparison.print();

    Ok(())
}

fn output_report(
    report: &TestReport,
    format: OutputFormat,
    file_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    match format {
        OutputFormat::Text => {
            report.print_text();
        }
        OutputFormat::Json => {
            let json = report.to_json()?;
            if let Some(path) = file_path {
                std::fs::write(path, &json)?;
                println!("Results written to {}", path);
            } else {
                println!("{}", json);
            }
        }
        OutputFormat::Csv => {
            let csv = report.to_csv()?;
            if let Some(path) = file_path {
                std::fs::write(path, &csv)?;
                println!("Results written to {}", path);
            } else {
                println!("{}", csv);
            }
        }
    }

    // Always save JSON if file path is specified but format is text
    if matches!(format, OutputFormat::Text) {
        if let Some(path) = file_path {
            let json = report.to_json()?;
            std::fs::write(path, &json)?;
            println!("Results written to {}", path);
        }
    }

    Ok(())
}

/// Trait for testers to allow generic progress handling
#[async_trait::async_trait]
trait Tester {
    async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
impl Tester for http::HttpTester {
    async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        http::HttpTester::run(self).await.map_err(|e| Box::new(e) as _)
    }
}

#[async_trait::async_trait]
impl Tester for websocket::WebSocketTester {
    async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        websocket::WebSocketTester::run(self)
            .await
            .map_err(|e| Box::new(e) as _)
    }
}

#[async_trait::async_trait]
impl Tester for grpc::GrpcTester {
    async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        grpc::GrpcTester::run(self).await.map_err(|e| Box::new(e) as _)
    }
}

#[async_trait::async_trait]
impl Tester for tcp::TcpTester {
    async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tcp::TcpTester::run(self).await.map_err(|e| Box::new(e) as _)
    }
}
