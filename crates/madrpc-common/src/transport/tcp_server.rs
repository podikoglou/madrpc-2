use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar};
use std::thread;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::protocol::{Request, Response};
use crate::protocol::error::{MadrpcError, Result};
use crate::transport::codec::PostcardCodec;

/// Maximum message size (100 MB)
const MAX_MESSAGE_SIZE: usize = 100 * 1024 * 1024;

/// Async TCP server for MaDRPC (for Orchestrator)
///
/// This server uses tokio for async I/O and is suitable for the orchestrator
/// which handles many concurrent connections efficiently.
pub struct TcpServer {
    listener: TcpListener,
}

impl TcpServer {
    /// Create a new TCP server bound to the specified address
    pub async fn new(bind_addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(bind_addr).await
            .map_err(|e| MadrpcError::Connection(format!("Failed to bind to {}: {}", bind_addr, e)))?;

        Ok(Self { listener })
    }

    /// Get the actual bound address
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.listener.local_addr()
            .map_err(|e| MadrpcError::Connection(format!("Failed to get local addr: {}", e)))
    }

    /// Run the server with the given request handler
    ///
    /// This accepts connections in a loop and spawns an async task for each connection.
    /// Each connection processes multiple requests (keep-alive) until closed.
    pub async fn run_with_handler<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Response>> + Send + 'static,
    {
        let handler = Arc::new(handler);

        loop {
            // Accept next connection
            let (stream, peer_addr) = self.listener.accept().await
                .map_err(|e| MadrpcError::Connection(format!("Failed to accept connection: {}", e)))?;

            eprintln!("Connection established from {}", peer_addr);

            let handler = handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, handler).await {
                    eprintln!("Connection error: {}", e);
                }
            });
        }
    }
}

/// Handle a single TCP connection (async)
///
/// Processes multiple requests until the connection is closed.
async fn handle_connection<F, Fut>(
    mut stream: TcpStream,
    handler: Arc<F>,
) -> Result<()>
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Response>> + Send + 'static,
{
    loop {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Connection closed by peer
                eprintln!("Connection closed by peer");
                return Ok(());
            }
            Err(e) => {
                return Err(MadrpcError::Connection(format!("Failed to read length: {}", e)));
            }
        }

        let len = u32::from_be_bytes(len_buf) as usize;

        // Validate length
        if len > MAX_MESSAGE_SIZE {
            return Err(MadrpcError::InvalidResponse(format!(
                "Message too large: {} bytes (max {} bytes)",
                len, MAX_MESSAGE_SIZE
            )));
        }

        // Read request data
        let mut buf = vec![0u8; len];
        match stream.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(e) => {
                return Err(MadrpcError::Connection(format!("Failed to read data: {}", e)));
            }
        }

        // Decode request
        let request = match PostcardCodec::decode_request(&buf) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Failed to decode request: {}", e);
                let error_response = Response::error(0, e.to_string());
                let _ = send_response(&mut stream, &error_response).await;
                continue;
            }
        };

        // Handle request
        let request_id = request.id;
        let response = match handler(request).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Handler error: {}", e);
                Response::error(request_id, e.to_string())
            }
        };

        // Send response
        if let Err(e) = send_response(&mut stream, &response).await {
            eprintln!("Failed to send response: {}", e);
            return Err(e);
        }
    }
}

/// Send a response with length prefix
async fn send_response(stream: &mut TcpStream, response: &Response) -> Result<()> {
    let encoded = PostcardCodec::encode_response(response)?;

    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes()).await
        .map_err(|e| MadrpcError::Connection(format!("Failed to send response length: {}", e)))?;
    stream.write_all(&encoded).await
        .map_err(|e| MadrpcError::Connection(format!("Failed to send response data: {}", e)))?;

    Ok(())
}

/// Simple semaphore implementation using std primitives
struct StdSemaphore {
    permits: AtomicUsize,
    max: usize,
    condvar: Condvar,
}

impl StdSemaphore {
    fn new(max: usize) -> Self {
        Self {
            permits: AtomicUsize::new(max),
            max,
            condvar: Condvar::new(),
        }
    }

    /// Acquire a permit, blocking until one is available
    fn acquire(&self) -> SemaphorePermit<'_> {
        // Only wait if we've exhausted permits
        while self.permits.load(Ordering::Acquire) == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }

        // Decrement permit count
        self.permits.fetch_sub(1, Ordering::AcqRel);

        SemaphorePermit {
            semaphore: self,
        }
    }
}

/// Permit that releases the semaphore when dropped
struct SemaphorePermit<'a> {
    semaphore: &'a StdSemaphore,
}

impl<'a> Drop for SemaphorePermit<'a> {
    fn drop(&mut self) {
        // Increment permit count
        self.semaphore.permits.fetch_add(1, Ordering::AcqRel);
        // Notify one waiting thread
        self.semaphore.condvar.notify_one();
    }
}

/// Threaded TCP server for MaDRPC (for Nodes)
///
/// This server uses std::net::TcpListener (blocking) and spawns OS threads
/// for each connection, limited by a semaphore. This is suitable for nodes
/// that need to run blocking QuickJS contexts.
pub struct TcpServerThreaded {
    listener: StdTcpListener,
    max_threads: usize,
    semaphore: Arc<StdSemaphore>,
}

impl TcpServerThreaded {
    /// Create a new threaded TCP server bound to the specified address
    ///
    /// `max_threads` controls the maximum number of concurrent connections.
    pub fn new(bind_addr: &str, max_threads: usize) -> Result<Self> {
        let listener = StdTcpListener::bind(bind_addr)
            .map_err(|e| MadrpcError::Connection(format!("Failed to bind to {}: {}", bind_addr, e)))?;

        // Set non-blocking for the parent thread
        listener.set_nonblocking(true)
            .map_err(|e| MadrpcError::Connection(format!("Failed to set non-blocking: {}", e)))?;

        let semaphore = Arc::new(StdSemaphore::new(max_threads));

        Ok(Self {
            listener,
            max_threads,
            semaphore,
        })
    }

    /// Get the actual bound address
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.listener.local_addr()
            .map_err(|e| MadrpcError::Connection(format!("Failed to get local addr: {}", e)))
    }

    /// Run the server with the given request handler
    ///
    /// The main thread accepts connections in a loop. For each connection:
    /// - Acquire a semaphore permit (blocks if max_threads reached)
    /// - Spawn a worker thread with the connection
    /// - Worker processes multiple requests (keep-alive)
    /// - Permit released when worker completes
    pub fn run_with_handler<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Result<Response> + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        let semaphore = self.semaphore.clone();

        loop {
            // Accept next connection (non-blocking with timeout)
            match self.listener.accept() {
                Ok((mut stream, peer_addr)) => {
                    eprintln!("Connection established from {}", peer_addr);

                    // Set stream back to blocking mode for the worker thread
                    stream.set_nonblocking(false)
                        .map_err(|e| MadrpcError::Connection(format!("Failed to set blocking mode: {}", e)))?;

                    // Acquire permit (may block if max_threads reached)
                    let _permit = semaphore.acquire();

                    let handler = handler.clone();
                    thread::spawn(move || {
                        // Handle the connection
                        if let Err(e) = handle_connection_threaded(stream, handler) {
                            eprintln!("Connection error: {}", e);
                        }
                        // Permit is released when dropped
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection ready, sleep a bit
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                    continue;
                }
            }
        }
    }
}

/// Handle a single TCP connection (threaded, blocking)
///
/// Processes multiple requests until the connection is closed.
fn handle_connection_threaded<F>(
    mut stream: std::net::TcpStream,
    handler: Arc<F>,
) -> Result<()>
where
    F: Fn(Request) -> Result<Response> + Send + Sync + 'static,
{
    loop {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Connection closed by peer
                eprintln!("Connection closed by peer");
                return Ok(());
            }
            Err(e) => {
                return Err(MadrpcError::Connection(format!("Failed to read length: {}", e)));
            }
        }

        let len = u32::from_be_bytes(len_buf) as usize;

        // Validate length
        if len > MAX_MESSAGE_SIZE {
            return Err(MadrpcError::InvalidResponse(format!(
                "Message too large: {} bytes (max {} bytes)",
                len, MAX_MESSAGE_SIZE
            )));
        }

        // Read request data
        let mut buf = vec![0u8; len];
        match stream.read_exact(&mut buf) {
            Ok(_) => {}
            Err(e) => {
                return Err(MadrpcError::Connection(format!("Failed to read data: {}", e)));
            }
        }

        // Decode request
        let request = match PostcardCodec::decode_request(&buf) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Failed to decode request: {}", e);
                let error_response = Response::error(0, e.to_string());
                let _ = send_response_threaded(&mut stream, &error_response);
                continue;
            }
        };

        // Handle request
        let request_id = request.id;
        let response = match handler(request) {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Handler error: {}", e);
                Response::error(request_id, e.to_string())
            }
        };

        // Send response
        if let Err(e) = send_response_threaded(&mut stream, &response) {
            eprintln!("Failed to send response: {}", e);
            return Err(e);
        }
    }
}

/// Send a response with length prefix (blocking)
fn send_response_threaded(stream: &mut std::net::TcpStream, response: &Response) -> Result<()> {
    let encoded = PostcardCodec::encode_response(response)?;

    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())
        .map_err(|e| MadrpcError::Connection(format!("Failed to send response length: {}", e)))?;
    stream.write_all(&encoded)
        .map_err(|e| MadrpcError::Connection(format!("Failed to send response data: {}", e)))?;
    stream.flush()
        .map_err(|e| MadrpcError::Connection(format!("Failed to flush stream: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tcp_server_creation() {
        let server = TcpServer::new("127.0.0.1:0").await;
        assert!(server.is_ok());
    }

    #[test]
    fn test_tcp_server_threaded_creation() {
        let server = TcpServerThreaded::new("127.0.0.1:0", 4);
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_tcp_server_local_addr() {
        let server = TcpServer::new("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr();
        assert!(addr.is_ok());
    }

    #[test]
    fn test_tcp_server_threaded_local_addr() {
        let server = TcpServerThreaded::new("127.0.0.1:0", 4).unwrap();
        let addr = server.local_addr();
        assert!(addr.is_ok());
    }
}
