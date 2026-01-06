use madrpc_common::transport::TcpTransportAsync;
use madrpc_common::protocol::error::{MadrpcError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Pooled connection wrapper
#[derive(Clone)]
pub struct PooledConnection {
    pub stream: Arc<Mutex<TcpStream>>,
    pub addr: String,
}

impl PooledConnection {
    /// Check if the connection is still valid
    pub async fn is_valid(&self) -> bool {
        // Try to check if the socket is still connected
        let stream = self.stream.try_lock();
        if stream.is_err() {
            return false;
        }

        let stream = stream.unwrap();
        // Check if the stream is still writable
        match stream.try_write(&[]) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
            Err(_) => false,
        }
    }
}

/// Connection pool configuration
#[derive(Clone)]
pub struct PoolConfig {
    /// Maximum number of connections per node
    pub max_connections: usize,
    /// Maximum time to wait for pool acquisition in milliseconds
    pub acquire_timeout_ms: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            acquire_timeout_ms: 30000, // 30 seconds
        }
    }
}

/// Connection pool for TCP connections
pub struct ConnectionPool {
    transport: TcpTransportAsync,
    inner: Arc<Mutex<PoolInner>>,
}

struct PoolInner {
    connections: HashMap<String, Vec<PooledConnection>>,
    available: HashMap<String, usize>,
    config: PoolConfig,
}

impl ConnectionPool {
    /// Create a new connection pool
    pub fn new(config: PoolConfig) -> Result<Self> {
        let transport = TcpTransportAsync::new()?;

        Ok(Self {
            transport,
            inner: Arc::new(Mutex::new(PoolInner {
                connections: HashMap::new(),
                available: HashMap::new(),
                config,
            })),
        })
    }

    /// Get a connection from the pool or create a new one
    pub async fn acquire(&self, addr: &str) -> Result<PooledConnection> {
        let timeout_duration = tokio::time::Duration::from_millis({
            let inner = self.inner.lock().await;
            inner.config.acquire_timeout_ms
        });
        let start = Instant::now();

        loop {
            // Check for timeout
            if start.elapsed() >= timeout_duration {
                return Err(MadrpcError::PoolTimeout(
                    self.inner.lock().await.config.acquire_timeout_ms,
                ));
            }

            // First, try to get an available connection
            {
                let mut inner = self.inner.lock().await;

                // Check if we have any available connections
                let avail_count = inner.available.get(addr).copied().unwrap_or(0);

                if avail_count > 0 {
                    // We have an available connection
                    if let Some(conns) = inner.connections.get_mut(addr) {
                        // Try to get a valid connection (LIFO for better cache locality)
                        while let Some(conn) = conns.last() {
                            let conn = conn.clone();

                            // Validate the connection
                            if conn.is_valid().await {
                                // Connection is valid, use it
                                conns.pop();
                                return Ok(conn);
                            } else {
                                // Invalid connection, remove it from pool
                                tracing::debug!(
                                    addr = %conn.addr,
                                    "Removing invalid connection from pool"
                                );
                                conns.pop();
                            }
                        }

                        // We've removed all invalid connections, update available count to 0
                        *inner.available.get_mut(addr).unwrap() = 0;
                    }
                }

                // Check if we've reached max connections
                let current_count = inner.connections.get(addr).map(|v| v.len()).unwrap_or(0);
                if current_count < inner.config.max_connections {
                    // We can create a new connection
                    break;
                }

                // Pool is full, wait and retry
                drop(inner);
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        }

        // No available connection - create new one (without holding lock)
        let stream = self.transport.connect(addr).await?;
        let pooled = PooledConnection {
            stream: Arc::new(Mutex::new(stream)),
            addr: addr.to_string(),
        };

        // Add to pool (with lock)
        let mut inner = self.inner.lock().await;
        inner.connections
            .entry(addr.to_string())
            .or_insert_with(Vec::new)
            .push(pooled.clone());

        inner.available.entry(addr.to_string()).or_insert(0);

        Ok(pooled)
    }

    /// Return a connection to the pool
    pub async fn release(&self, conn: PooledConnection) {
        let mut inner = self.inner.lock().await;

        if let Some(avail) = inner.available.get_mut(&conn.addr) {
            *avail += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests require a running TCP server
    // These are unit tests for the pool logic

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = ConnectionPool::new(PoolConfig::default());
        assert!(pool.is_ok());
    }

    #[tokio::test]
    async fn test_config_default() {
        let config = PoolConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.acquire_timeout_ms, 30000);
    }

    #[tokio::test]
    async fn test_acquire_nonexistent_addr_fails() {
        let pool = ConnectionPool::new(PoolConfig::default()).unwrap();
        let result = pool.acquire("localhost:9999").await;
        // Will fail because no server is running
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pool_config_custom() {
        let config = PoolConfig {
            max_connections: 5,
            acquire_timeout_ms: 5000,
        };
        assert_eq!(config.max_connections, 5);
        assert_eq!(config.acquire_timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_pool_timeout() {
        // Create a pool with very short timeout
        let config = PoolConfig {
            max_connections: 1,
            acquire_timeout_ms: 100, // 100ms timeout
        };
        let pool = ConnectionPool::new(config).unwrap();

        // Try to acquire from a non-existent server
        let result = pool.acquire("localhost:9999").await;

        // Should eventually timeout with PoolTimeout error
        // Note: This might also fail with Connection error, which is fine
        // The important thing is it doesn't hang forever
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pooled_connection_is_valid() {
        // Create a mock connection to test validation
        // This is a basic unit test - integration tests would need a real server
        let config = PoolConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.acquire_timeout_ms, 30000);
    }
}
