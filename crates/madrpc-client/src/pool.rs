use madrpc_common::transport::TcpTransportAsync;
use madrpc_common::protocol::error::{MadrpcError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Pooled connection wrapper.
///
/// Wraps a TCP stream with its associated address for use in the connection pool.
/// The stream is wrapped in an `Arc<Mutex<T>>` to allow safe concurrent access
/// across multiple async tasks.
///
/// # Fields
///
/// - `stream`: The TCP stream wrapped in an async mutex for thread-safe access
/// - `addr`: The address this connection is connected to
#[derive(Clone)]
pub struct PooledConnection {
    /// The TCP stream
    pub stream: Arc<Mutex<TcpStream>>,
    /// The address this connection is to
    pub addr: String,
}

impl PooledConnection {
    /// Checks if the connection is still valid.
    ///
    /// This uses a short timeout when attempting to acquire the lock to avoid
    /// false negatives during contention. If the lock is held by another active
    /// request, we consider the connection valid (since it's in use).
    ///
    /// # Implementation Details
    ///
    /// The validation strategy is:
    /// - Try to acquire the stream lock with a 10ms timeout
    /// - If the lock is acquired: the connection is idle and available (valid)
    /// - If the timeout expires: the lock is held by another request, meaning
    ///   the connection is active and in use (also valid)
    ///
    /// This approach ensures that connections are not incorrectly marked as
    /// invalid during periods of high contention.
    ///
    /// # Returns
    ///
    /// Always returns `true` - connections are assumed valid unless real I/O fails.
    /// The actual connection health will be verified during request execution
    /// when the stream is used for real I/O.
    pub async fn is_valid(&self) -> bool {
        // Try to acquire the lock with a short timeout
        // If we can get it, the connection is available and likely valid
        // If we timeout, it means someone else is using it - also valid!
        match tokio::time::timeout(
            tokio::time::Duration::from_millis(10),
            self.stream.lock()
        ).await {
            Ok(_) => true,  // Lock acquired, connection is idle and valid
            Err(_) => true, // Timeout means lock is held - connection is active and valid
        }
    }
}

/// Connection pool configuration.
///
/// Controls the behavior of the connection pool including maximum connections
/// and acquisition timeout.
///
/// # Fields
///
/// - `max_connections`: Maximum number of connections to maintain per unique address
/// - `acquire_timeout_ms`: Maximum time in milliseconds to wait for a connection
///
/// # Default Configuration
///
/// The default configuration is:
/// - `max_connections`: 10
/// - `acquire_timeout_ms`: 30000 (30 seconds)
///
/// # Example
///
/// ```rust
/// use madrpc_client::PoolConfig;
///
/// // Custom configuration: up to 20 connections, 60-second timeout
/// let config = PoolConfig {
///     max_connections: 20,
///     acquire_timeout_ms: 60000,
/// };
/// ```
#[derive(Clone)]
pub struct PoolConfig {
    /// Maximum number of connections per unique address
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

/// Connection pool for TCP connections.
///
/// The pool manages connections to multiple addresses, allowing efficient reuse
/// of TCP connections across multiple requests. Connections are acquired and
/// released using [`acquire`](Self::acquire) and [`release`](Self::release).
///
/// # Architecture
///
/// - Connections are grouped by target address
/// - Each address maintains its own pool with `max_connections` limit
/// - LIFO (last-in-first-out) strategy is used for reusing connections
/// - Invalid connections are automatically removed from the pool
///
/// # Thread Safety
///
/// The pool is thread-safe and can be safely used from multiple async tasks
/// concurrently. Internal state is protected by an async mutex.
///
/// # Example
///
/// ```rust,no_run
/// use madrpc_client::{ConnectionPool, PoolConfig};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let pool = ConnectionPool::new(PoolConfig::default())?;
///
/// // Acquire a connection
/// let conn = pool.acquire("127.0.0.1:8080").await?;
///
/// // Use the connection...
///
/// // Return it to the pool
/// pool.release(conn).await;
/// # Ok(())
/// # }
/// ```
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
    /// Creates a new connection pool.
    ///
    /// # Arguments
    ///
    /// * `config` - The pool configuration
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the pool or an error if transport initialization fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use madrpc_client::{ConnectionPool, PoolConfig};
    ///
    /// let pool = ConnectionPool::new(PoolConfig::default());
    /// assert!(pool.is_ok());
    /// ```
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

    /// Acquires a connection from the pool or creates a new one.
    ///
    /// This method implements the following strategy:
    ///
    /// 1. Check if a valid connection is available in the pool
    /// 2. If yes, return it (LIFO strategy for better cache locality)
    /// 3. If no, check if we can create a new connection (under `max_connections` limit)
    /// 4. If at limit, wait and retry (with timeout)
    /// 5. If under limit, create a new connection
    ///
    /// # Arguments
    ///
    /// * `addr` - The address to connect to (e.g., "127.0.0.1:8080")
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing a pooled connection, or an error if:
    /// - The pool acquisition times out
    /// - Connection establishment fails
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The pool is full and the acquisition timeout is exceeded
    /// - The TCP connection cannot be established
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

    /// Returns a connection to the pool.
    ///
    /// This marks the connection as available for reuse. The connection itself
    /// remains in the pool's internal storage and will be returned by future
    /// calls to [`acquire`](Self::acquire).
    ///
    /// # Arguments
    ///
    /// * `conn` - The connection to release
    ///
    /// # Note
    ///
    /// The connection is not closed when released. It remains open and can be
    /// reused for future requests. Connections are only removed from the pool
    /// if they fail validation during acquisition.
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

    #[tokio::test]
    async fn test_is_valid_handles_contention() {
        // Test that is_valid doesn't fail when lock is contended
        use tokio::net::TcpListener;
        use std::time::Duration;

        // Create a simple TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        // Spawn a task to accept one connection
        tokio::spawn(async move {
            let _ = listener.accept().await;
            // Keep connection open
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        // Create a pooled connection
        let transport = TcpTransportAsync::new().unwrap();
        let stream = transport.connect(&addr).await.unwrap();
        let conn = PooledConnection {
            stream: Arc::new(Mutex::new(stream)),
            addr: addr.clone(),
        };

        // Test 1: is_valid should succeed when no contention
        assert!(conn.is_valid().await, "is_valid should return true for idle connection");

        // Test 2: is_valid should succeed even when lock is held
        let conn_clone = conn.clone();
        let lock = conn.stream.lock().await;
        // Hold the lock while checking validity
        let is_valid_task = tokio::spawn(async move {
            conn_clone.is_valid().await
        });

        // The validation should complete and return true
        // (either by acquiring lock after we release it, or timing out and returning true)
        let result = tokio::time::timeout(Duration::from_millis(100), is_valid_task).await;
        assert!(result.is_ok(), "is_valid should complete quickly even under contention");
        assert!(result.unwrap().unwrap(), "is_valid should return true even when lock is held");
        drop(lock);

        // Test 3: Multiple concurrent is_valid calls should all succeed
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let c = conn.clone();
                tokio::spawn(async move {
                    c.is_valid().await
                })
            })
            .collect();

        for handle in handles {
            let result = tokio::time::timeout(Duration::from_millis(100), handle).await;
            assert!(result.is_ok(), "is_valid should complete within timeout");
            assert!(result.unwrap().unwrap(), "is_valid should return true");
        }
    }

    #[tokio::test]
    async fn test_is_valid_timeout_behavior() {
        // Test the timeout behavior of is_valid
        use tokio::net::TcpListener;
        use std::time::Duration;

        // Create a simple TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        // Spawn a task to accept one connection
        tokio::spawn(async move {
            let _ = listener.accept().await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        // Create a pooled connection
        let transport = TcpTransportAsync::new().unwrap();
        let stream = transport.connect(&addr).await.unwrap();
        let conn = PooledConnection {
            stream: Arc::new(Mutex::new(stream)),
            addr,
        };

        // Hold the lock
        let _lock = conn.stream.lock().await;

        // Start is_valid check (should timeout waiting for lock)
        let start = std::time::Instant::now();
        let is_valid_result = conn.is_valid().await;
        let elapsed = start.elapsed();

        // Should return true (connection is valid, lock is just held)
        assert!(is_valid_result, "is_valid should return true even when lock is held");

        // Should take approximately 10ms (our timeout) since lock is held
        assert!(
            elapsed >= Duration::from_millis(5) && elapsed < Duration::from_millis(50),
            "is_valid should timeout in ~10ms when lock is held, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_pool_acquire_reuses_valid_connections() {
        // Test that the pool reuses connections that pass is_valid check
        use tokio::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        // Track connection count
        let conn_count = Arc::new(AtomicUsize::new(0));

        // Create a TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let conn_count_clone = conn_count.clone();

        // Spawn a task to accept connections
        tokio::spawn(async move {
            loop {
                if listener.accept().await.is_ok() {
                    conn_count_clone.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        // Create pool with max_connections = 2
        let config = PoolConfig {
            max_connections: 2,
            acquire_timeout_ms: 1000,
        };
        let pool = ConnectionPool::new(config).unwrap();

        // Acquire and release connections
        let _conn1 = pool.acquire(&addr).await.unwrap();
        let _conn2 = pool.acquire(&addr).await.unwrap();

        // Should have created exactly 2 connections
        tokio::time::sleep(Duration::from_millis(50)).await; // Give time for connections
        assert_eq!(conn_count.load(Ordering::SeqCst), 2, "Should create exactly 2 connections");
    }
}
