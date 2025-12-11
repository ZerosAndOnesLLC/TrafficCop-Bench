//! High-performance in-memory HTTP backend server for benchmarking
//!
//! Uses multiple accept loops for maximum throughput.

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use socket2::{Domain, Protocol, Socket, Type};
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Pre-allocated response - zero allocation per request
static RESPONSE_BODY: Bytes = Bytes::from_static(b"{\"status\":\"ok\"}");

/// Backend server configuration
#[derive(Clone)]
pub struct BackendConfig {
    pub addr: SocketAddr,
    pub response_body: Bytes,
    pub status_code: StatusCode,
    pub num_workers: usize,
}

impl Default for BackendConfig {
    fn default() -> Self {
        let num_workers = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4);
        Self {
            addr: "127.0.0.1:9000".parse().unwrap(),
            response_body: RESPONSE_BODY.clone(),
            status_code: StatusCode::OK,
            num_workers,
        }
    }
}

/// Handle request with zero allocation
#[inline(always)]
async fn handle_request(
    _req: Request<Incoming>,
    response_body: Bytes,
    status: StatusCode,
) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("content-length", response_body.len())
        .body(Full::new(response_body))
        .unwrap())
}

/// Backend server handle
pub struct BackendServer {
    shutdown_tx: Option<oneshot::Sender<()>>,
    #[allow(dead_code)]
    addr: SocketAddr,
}

impl BackendServer {
    /// Start backend with multiple accept loops using SO_REUSEPORT
    pub async fn start(config: BackendConfig) -> std::io::Result<Self> {
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let addr = config.addr;

        // Spawn multiple accept loops
        for _ in 0..config.num_workers {
            let cfg = config.clone();

            tokio::spawn(async move {
                // Create socket with SO_REUSEPORT
                let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
                socket.set_reuse_address(true).unwrap();
                #[cfg(unix)]
                socket.set_reuse_port(true).unwrap();
                socket.set_nonblocking(true).unwrap();
                socket.bind(&cfg.addr.into()).unwrap();
                socket.listen(8192).unwrap();

                let listener = TcpListener::from_std(socket.into()).unwrap();

                loop {
                    match listener.accept().await {
                        Ok((stream, _)) => {
                            stream.set_nodelay(true).ok();
                            let io = TokioIo::new(stream);
                            let body = cfg.response_body.clone();
                            let status = cfg.status_code;

                            tokio::spawn(async move {
                                let service = service_fn(move |req| {
                                    handle_request(req, body.clone(), status)
                                });

                                let _ = http1::Builder::new()
                                    .serve_connection(io, service)
                                    .await;
                            });
                        }
                        Err(_) => continue,
                    }
                }
            });
        }

        // Wait a bit for listeners to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
            addr,
        })
    }

    #[allow(dead_code)]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for BackendServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}
