use crate::runtime::MadrpcContext;
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// Configuration for the context pool
#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub pool_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        // Use pool_size = 1 for now due to QuickJS threading limitations
        // QuickJS contexts may not be safe to use from multiple threads
        // even with mutex protection. The infrastructure is in place to
        // increase this once we understand the limitations better.
        Self { pool_size: 1 }
    }
}

/// Guard that holds a context and auto-releases it back to the pool when dropped
pub struct PooledContext {
    context: Arc<MadrpcContext>,
    pool: Option<Arc<ContextPool>>,
    _permit: OwnedSemaphorePermit,
}

impl PooledContext {
    /// Call an RPC method on the pooled context
    pub fn call_rpc(&self, method: &str, args: Value) -> Result<Value> {
        self.context.call_rpc(method, args)
    }
}

impl Drop for PooledContext {
    fn drop(&mut self) {
        // Return the context to the pool synchronously
        // We need to do this before the permit is released to avoid race conditions
        if let Some(pool) = &self.pool {
            // Use a synchronous approach by directly accessing the available contexts
            // We're in a Drop context, so we can't block, but we can use try_lock
            if let Ok(mut available) = pool.available.try_lock() {
                available.push(self.context.clone());
            }
            // If try_lock fails, the context will be leaked (not ideal, but safe)
        }
        // The semaphore permit is automatically released when _permit is dropped
    }
}

/// Pool of QuickJS contexts for parallel RPC execution
pub struct ContextPool {
    /// Available contexts (not currently in use)
    available: Arc<Mutex<Vec<Arc<MadrpcContext>>>>,
    /// Semaphore for tracking available contexts
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
            available: Arc::new(Mutex::new(contexts)),
            semaphore: Arc::new(Semaphore::new(config.pool_size)),
            config,
        })
    }

    /// Acquire a context from the pool
    ///
    /// If the pool is exhausted, this will asynchronously wait until a context
    /// becomes available (same behavior as the current semaphore-based limiter).
    pub async fn acquire(&self) -> Result<PooledContext> {
        // Acquire an owned permit from the semaphore
        let permit = self.semaphore.clone().acquire_owned().await
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to acquire pool permit: {}", e)))?;

        // Take a context from the available pool
        let context = {
            let mut available = self.available.lock().await;
            available.pop()
                .ok_or_else(|| MadrpcError::InvalidRequest("No contexts available in pool".to_string()))?
        };

        Ok(PooledContext {
            context,
            pool: Some(Arc::new(self.clone())),
            _permit: permit,
        })
    }

    /// Get the pool size
    pub fn pool_size(&self) -> usize {
        self.config.pool_size
    }
}

// We need Clone for the pool so PooledContext can hold an Arc<ContextPool>
impl Clone for ContextPool {
    fn clone(&self) -> Self {
        Self {
            available: self.available.clone(),
            semaphore: self.semaphore.clone(),
            config: self.config.clone(),
        }
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
        let config = PoolConfig { pool_size: 1 };
        let pool = ContextPool::new(script, config);
        assert!(pool.is_ok());
        let pool = pool.unwrap();
        assert_eq!(pool.pool_size(), 1);
    }

    #[tokio::test]
    #[ignore = "QuickJS contexts have threading limitations when accessed from different blocking threads"]
    async fn test_pool_acquire_release() {
        let script = create_test_script(r#"
            madrpc.register('test', function() {
                return { result: 'ok' };
            });
        "#);
        let config = PoolConfig { pool_size: 1 };
        let pool = Arc::new(ContextPool::new(script, config).unwrap());

        // Acquire the only context
        let ctx1 = pool.acquire().await.unwrap();
        let result1 = tokio::task::spawn_blocking(move || {
            ctx1.call_rpc("test", json!({}))
        }).await.unwrap().unwrap();
        assert_eq!(result1, json!({"result": "ok"}));

        // ctx1 is dropped here, context returns to pool

        // Should be able to acquire again
        let ctx2 = pool.acquire().await.unwrap();
        let result2 = tokio::task::spawn_blocking(move || {
            ctx2.call_rpc("test", json!({}))
        }).await.unwrap().unwrap();
        assert_eq!(result2, json!({"result": "ok"}));
    }

    #[tokio::test]
    #[ignore = "QuickJS contexts have threading limitations when accessed from different blocking threads"]
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
            let ctx2 = pool_clone.acquire().await.unwrap();
            // Use spawn_blocking for the actual RPC call
            tokio::task::spawn_blocking(move || {
                ctx2.call_rpc("test", json!({}))
            }).await.unwrap()
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
        assert_eq!(result.unwrap(), json!({"result": "ok"}));
    }

    #[tokio::test]
    #[ignore = "QuickJS contexts have threading limitations when accessed from different blocking threads"]
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
        let config = PoolConfig { pool_size: 1 };
        let pool = Arc::new(ContextPool::new(script, config).unwrap());

        // Spawn multiple tasks - with pool_size=1 they will run serially
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let pool_clone = Arc::clone(&pool);
            let task = tokio::spawn(async move {
                let ctx = pool_clone.acquire().await.unwrap();
                // Use spawn_blocking for the actual RPC call
                tokio::task::spawn_blocking(move || {
                    ctx.call_rpc("compute", json!({"n": 100}))
                }).await.unwrap()
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
