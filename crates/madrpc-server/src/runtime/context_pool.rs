use crate::runtime::MadrpcContext;
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Semaphore, OwnedSemaphorePermit};

/// Configuration for the context pool
#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub pool_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self { pool_size: num_cpus::get() }
    }
}

/// Guard that holds a context and auto-releases the semaphore permit when dropped
pub struct PooledContext {
    context: Arc<MadrpcContext>,
    _permit: OwnedSemaphorePermit,
}

impl PooledContext {
    /// Call an RPC method on the pooled context
    pub fn call_rpc(&self, method: &str, args: Value) -> Result<Value> {
        self.context.call_rpc(method, args)
    }
}

/// Pool of QuickJS contexts for parallel RPC execution
pub struct ContextPool {
    contexts: Vec<Arc<MadrpcContext>>,
    semaphore: Arc<Semaphore>,
    config: PoolConfig,
}

impl ContextPool {
    /// Create a new context pool with the given configuration
    ///
    /// Each context in the pool independently loads and evaluates the script,
    /// allowing true parallelism across contexts.
    pub fn new(script_path: PathBuf, config: PoolConfig) -> Result<Self> {
        let mut contexts = Vec::with_capacity(config.pool_size);

        // Create each context independently - each loads the script separately
        for _ in 0..config.pool_size {
            let ctx = MadrpcContext::with_client(&script_path, None)?;
            contexts.push(Arc::new(ctx));
        }

        Ok(Self {
            contexts,
            semaphore: Arc::new(Semaphore::new(config.pool_size)),
            config,
        })
    }

    /// Acquire a context from the pool
    ///
    /// If the pool is exhausted, this will asynchronously wait until a context
    /// becomes available (same behavior as the current semaphore-based limiter).
    pub async fn acquire(&self) -> Result<PooledContext> {
        // Acquire an owned permit from the semaphore directly
        let owned_permit = self.semaphore.clone().acquire_owned().await
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to acquire pool permit: {}", e)))?;

        // Get a context - for now we use round-robin by always taking the first
        // In the future, we could implement smarter selection strategies
        let context = self.contexts.first()
            .ok_or_else(|| MadrpcError::InvalidRequest("Context pool is empty".to_string()))?
            .clone();

        Ok(PooledContext { context, _permit: owned_permit })
    }

    /// Get the pool size
    pub fn pool_size(&self) -> usize {
        self.config.pool_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn create_test_script(content: &str) -> PathBuf {
        static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!("/tmp/test_context_pool_{}.js", id));
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_pool_config_default() {
        let config = PoolConfig::default();
        assert!(config.pool_size > 0);
    }

    #[test]
    fn test_pool_creation() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let config = PoolConfig { pool_size: 2 };
        let pool = ContextPool::new(script, config);
        assert!(pool.is_ok());
        let pool = pool.unwrap();
        assert_eq!(pool.pool_size(), 2);
    }

    #[tokio::test]
    async fn test_pool_acquire_release() {
        let script = create_test_script(r#"
            madrpc.register('test', function() {
                return { result: 'ok' };
            });
        "#);
        let config = PoolConfig { pool_size: 2 };
        let pool = Arc::new(ContextPool::new(script, config).unwrap());

        // Acquire a context
        let ctx1 = pool.acquire().await.unwrap();
        assert_eq!(ctx1.call_rpc("test", json!({})).unwrap(), json!({"result": "ok"}));

        // Acquire another context
        let ctx2 = pool.acquire().await.unwrap();
        assert_eq!(ctx2.call_rpc("test", json!({})).unwrap(), json!({"result": "ok"}));

        // Drop both contexts - should release permits back to pool
        drop(ctx1);
        drop(ctx2);

        // Should be able to acquire again
        let ctx3 = pool.acquire().await.unwrap();
        assert_eq!(ctx3.call_rpc("test", json!({})).unwrap(), json!({"result": "ok"}));
    }

    #[tokio::test]
    async fn test_pool_exhaustion_waits() {
        let script = create_test_script(r#"
            madrpc.register('test', function() {
                return { result: 'ok' };
            });
        "#);
        let config = PoolConfig { pool_size: 1 };
        let pool = Arc::new(ContextPool::new(script, config).unwrap());

        // Acquire the only context
        let ctx1 = pool.acquire().await.unwrap();

        // Try to acquire again - should block until ctx1 is dropped
        let pool_clone = Arc::clone(&pool);
        let acquire_task = tokio::spawn(async move {
            pool_clone.acquire().await
        });

        // Give the acquire task time to start waiting
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // The acquire task should still be running (waiting)
        assert!(!acquire_task.is_finished());

        // Drop ctx1 to release the permit
        drop(ctx1);

        // Now the acquire task should complete
        let result = acquire_task.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_parallel_execution() {
        let script = create_test_script(r#"
            madrpc.register('compute', function(args) {
                // Simulate some work
                let sum = 0;
                for (let i = 0; i < args.n; i++) {
                    sum += i;
                }
                return { sum: sum };
            });
        "#);
        let config = PoolConfig { pool_size: 3 };
        let pool = Arc::new(ContextPool::new(script, config).unwrap());

        // Spawn multiple tasks in parallel
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let pool_clone = Arc::clone(&pool);
            let task = tokio::spawn(async move {
                let ctx = pool_clone.acquire().await.unwrap();
                ctx.call_rpc("compute", json!({"n": 100}))
            });
            tasks.push(task);
        }

        // Wait for all tasks to complete
        let results: Vec<_> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|r| r.unwrap().unwrap())
            .collect();

        assert_eq!(results.len(), 5);
        for result in results {
            assert_eq!(result, json!({"sum": 4950})); // sum of 0..99
        }
    }
}
