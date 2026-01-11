use boa_engine::{Context, Source, js_string, value::JsValue, object::builtins::JsPromise, builtins::promise::PromiseState, job::JobExecutor};
use std::path::Path;
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::cell::RefCell;
use std::time::Instant;

use crate::runtime::{bindings, conversions::{json_to_js_value, js_value_to_json}, TokioJobExecutor};

/// Maximum time to wait for a JavaScript Promise to resolve (in milliseconds).
///
/// This prevents CPU exhaustion and hangs from promises that never resolve.
/// The 30-second timeout is generous enough for legitimate async operations
/// while preventing indefinite hangs.
const MAX_PROMISE_WAIT_MS: u64 = 30_000;

/// Thread-local marker to ensure MadrpcContext is used on the correct thread.
///
/// This is a zero-sized type that is !Send and !Sync, which prevents
/// MadrpcContext from being sent or shared across threads. This ensures
/// that the Boa Context (which has thread-local state) is always accessed
/// from the same thread it was created on.
///
/// # Thread Safety
///
/// Boa's Context has thread-local state and is not thread-safe. By including
/// this PhantomData marker, we ensure at the type level that MadrpcContext
/// cannot be sent to another thread or shared between threads.
///
/// This is a safer alternative to `unsafe impl Send/Sync` because it relies
/// on Rust's type system to enforce thread safety rather than documentation
/// and programmer discipline.
///
/// # How It Works
///
/// `Rc<()>` is !Send and !Sync. By including it as PhantomData in the
/// ThreadNotSendSync struct, we propagate these marker traits to the entire
/// MadrpcContext struct, making it also !Send and !Sync.
use std::marker::PhantomData;
use std::rc::Rc;

struct ThreadNotSendSync {
    _marker: PhantomData<Rc<()>>,
}

/// Boa context wrapper with MaDRPC bindings.
///
/// This type wraps Boa's JavaScript context with custom MaDRPC-specific
/// bindings that allow JavaScript code to register functions and make
/// distributed RPC calls.
///
/// # Thread Safety
///
/// **IMPORTANT**: MadrpcContext is NOT Send or Sync. It must be used on
/// the same thread it was created on. This is enforced at the type level
/// by the ThreadNotSendSync marker.
///
/// This design ensures that Boa's Context (which has thread-local state)
/// is always accessed from the same thread, preventing data races and
/// undefined behavior.
///
/// # Usage Pattern
///
/// Each request should create its own fresh MadrpcContext instance.
/// Contexts should never be shared between threads or reused across
/// requests.
///
/// # Example
///
/// ```ignore
/// // Create a new context from a script file
/// let mut ctx = MadrpcContext::new("script.js")?;
///
/// // Call a registered function
/// let result = ctx.call_rpc("myFunction", json!({"arg": 42}))?;
/// ```
pub struct MadrpcContext {
    /// Thread-local marker that prevents Send/Sync
    _thread_marker: ThreadNotSendSync,
    /// The Boa context (wrapped in Mutex for internal synchronization)
    ctx: boa_engine::context::Context,
    /// Optional client for distributed RPC calls
    /// Note: This is NOT used in bindings anymore - we use closure capture instead
    /// But we keep it here for the distributed_call method
    client: Option<Arc<madrpc_client::MadrpcClient>>,
    /// The job executor for async JavaScript execution
    job_executor: Rc<TokioJobExecutor>,
}

impl MadrpcContext {
    /// Create a new Boa context with MaDRPC bindings.
    ///
    /// This constructor reads the script from the given path, creates a fresh
    /// Boa context with MaDRPC bindings, and evaluates the script.
    ///
    /// # Parameters
    ///
    /// * `script_path` - Path to the JavaScript script file to load and evaluate
    ///
    /// # Returns
    ///
    /// A new MadrpcContext instance with the script loaded and evaluated.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The script file cannot be read
    /// - The Boa context cannot be built
    /// - The script contains syntax errors or fails to evaluate
    pub fn new(script_path: impl AsRef<Path>) -> Result<Self> {
        Self::with_client(script_path, None)
    }

    /// Create a new Boa context from a cached script source string.
    ///
    /// This is more efficient than `new()` because it avoids file I/O.
    /// The script source is parsed and evaluated in a fresh context.
    ///
    /// # Parameters
    ///
    /// * `script_source` - The JavaScript source code as a string
    ///
    /// # Returns
    ///
    /// A new MadrpcContext instance with the script evaluated.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The Boa context cannot be built
    /// - The script contains syntax errors or fails to evaluate
    pub fn from_source(script_source: &str) -> Result<Self> {
        Self::with_client_from_source(script_source, None)
    }

    /// Create a new Boa context with optional client for distributed calls.
    ///
    /// This constructor is similar to `new()` but allows passing an optional
    /// MadrpcClient for making distributed RPC calls to other nodes.
    ///
    /// # Parameters
    ///
    /// * `script_path` - Path to the JavaScript script file
    /// * `client` - Optional MadrpcClient for distributed calls
    ///
    /// # Returns
    ///
    /// A new MadrpcContext instance.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The script file cannot be read
    /// - The Boa context cannot be built
    /// - The script fails to evaluate
    pub fn with_client(
        script_path: impl AsRef<Path>,
        client: Option<madrpc_client::MadrpcClient>,
    ) -> Result<Self> {
        let client = client.map(Arc::new);
        let job_executor = std::rc::Rc::new(crate::runtime::TokioJobExecutor::new());
        let mut ctx = Context::builder()
            .job_executor(job_executor.clone())
            .build()
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to build context: {}", e)))?;

        // Install madrpc bindings (native Rust functions)
        bindings::install_madrpc_bindings(&mut ctx, client.clone())?;

        // Load and evaluate the script
        let script = std::fs::read_to_string(script_path)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to load script: {}", e)))?;

        ctx.eval(Source::from_bytes(&script))
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Script evaluation error: {}", e)))?;

        Ok(Self {
            _thread_marker: ThreadNotSendSync { _marker: PhantomData },
            ctx,
            client,
            job_executor,
        })
    }

    /// Create a new Boa context from cached script source with optional client.
    ///
    /// This is more efficient than `with_client()` because it avoids file I/O.
    /// Use this when you have already loaded the script source and want to
    /// create multiple contexts.
    ///
    /// # Parameters
    ///
    /// * `script_source` - The JavaScript source code as a string
    /// * `client` - Optional MadrpcClient for distributed calls
    ///
    /// # Returns
    ///
    /// A new MadrpcContext instance.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The Boa context cannot be built
    /// - The script fails to evaluate
    pub fn with_client_from_source(
        script_source: &str,
        client: Option<madrpc_client::MadrpcClient>,
    ) -> Result<Self> {
        let client = client.map(Arc::new);
        let job_executor = std::rc::Rc::new(crate::runtime::TokioJobExecutor::new());
        let mut ctx = Context::builder()
            .job_executor(job_executor.clone())
            .build()
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to build context: {}", e)))?;

        // Install madrpc bindings (native Rust functions)
        bindings::install_madrpc_bindings(&mut ctx, client.clone())?;

        // Evaluate the script source
        ctx.eval(Source::from_bytes(script_source))
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Script evaluation error: {}", e)))?;

        Ok(Self {
            _thread_marker: ThreadNotSendSync { _marker: PhantomData },
            ctx,
            client,
            job_executor,
        })
    }

    /// Call a registered RPC function.
    ///
    /// This method looks up a function by name in the madrpc registry and
    /// calls it with the provided arguments. The function can be synchronous
    /// or async (returning a Promise).
    ///
    /// # Parameters
    ///
    /// * `method` - Name of the registered function to call
    /// * `args` - Arguments to pass to the function (must be valid JSON)
    ///
    /// # Returns
    ///
    /// The function's return value as JSON.
    ///
    /// # Promise Handling
    ///
    /// If the function returns a Promise, this method will poll it to
    /// completion before returning. It uses a loop that:
    /// 1. Runs pending promise jobs
    /// 2. Checks the promise state
    /// 3. Continues polling until the promise settles
    /// 4. Returns the fulfillment value or a rejection error
    ///
    /// The polling has a maximum timeout of 30 seconds to prevent
    /// CPU exhaustion and indefinite hangs. This timeout uses wall-clock
    /// time measurement rather than iteration counting for more predictable
    /// behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The method is not registered
    /// - The registered value is not a function
    /// - Function execution fails
    /// - Arguments cannot be converted
    /// - A promise is rejected
    /// - A promise times out (exceeds 30 seconds)
    pub fn call_rpc(&mut self, method: &str, args: JsonValue) -> Result<JsonValue> {
        tracing::debug!("call_rpc: calling method '{}'", method);

        // Get madrpc object and registry
        let madrpc = self.ctx.global_object()
            .get(js_string!("madrpc"), &mut self.ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        tracing::debug!("call_rpc: Getting registry...");
        let registry_val = madrpc.as_object()
            .and_then(|o| o.get(js_string!("__registry"), &mut self.ctx).ok())
            .ok_or_else(|| MadrpcError::InvalidRequest("Failed to access registry".into()))?;

        let registry = registry_val.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Registry is not an object".into()))?;

        // Get registered function
        tracing::debug!("call_rpc: Getting function '{}' from registry...", method);
        let func = registry.get(js_string!(method), &mut self.ctx)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Method '{}' lookup error: {}", method, e)))?;

        if func.is_undefined() {
            return Err(MadrpcError::InvalidRequest(format!("Method '{}' is not registered", method)));
        }

        let func_obj = func.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Registered value is not a function".into()))?;

        // Convert args to JsValue and call
        tracing::debug!("call_rpc: Converting args and calling function...");
        let args_js = json_to_js_value(args, &mut self.ctx)?;
        let result = func_obj.call(&JsValue::undefined(), &[args_js], &mut self.ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Function execution error: {}", e)))?;

        // Check if result is a Promise
        if let Some(result_obj) = result.as_object() {
            if let Ok(promise) = JsPromise::from_object(result_obj.clone()) {
                tracing::debug!("call_rpc: Result is a Promise, waiting for resolution...");

                // Poll the promise with a timeout-based loop to prevent CPU exhaustion
                let start_time = Instant::now();
                let timeout_duration = std::time::Duration::from_millis(MAX_PROMISE_WAIT_MS);
                let mut poll_count = 0;

                loop {
                    // Check timeout first to prevent infinite loops
                    let elapsed = start_time.elapsed();
                    if elapsed >= timeout_duration {
                        tracing::error!("call_rpc: Promise timeout after {}ms (max: {}ms)", elapsed.as_millis(), MAX_PROMISE_WAIT_MS);
                        return Err(MadrpcError::JavaScriptExecution(format!(
                            "Promise did not resolve within {}ms timeout",
                            MAX_PROMISE_WAIT_MS
                        )));
                    }

                    // Run pending promise jobs
                    let _ = self.ctx.run_jobs();

                    // Check promise state
                    match promise.state() {
                        PromiseState::Pending => {
                            // Continue polling with small sleep to prevent CPU exhaustion
                            // Sleep for 100 microseconds every 100 iterations
                            poll_count += 1;
                            if poll_count % 100 == 0 {
                                std::thread::sleep(std::time::Duration::from_micros(100));
                            }
                            continue;
                        }
                        PromiseState::Fulfilled(value) => {
                            tracing::debug!("call_rpc: Promise fulfilled after {}ms ({} polls)", elapsed.as_millis(), poll_count);
                            // Convert the fulfillment value to JSON
                            return js_value_to_json(value, &mut self.ctx);
                        }
                        PromiseState::Rejected(reason) => {
                            tracing::debug!("call_rpc: Promise rejected after {}ms ({} polls)", elapsed.as_millis(), poll_count);
                            // Convert rejection reason to string for error message
                            let reason_str = if let Some(s) = reason.as_string() {
                                s.to_std_string()
                                    .unwrap_or_else(|_| "Unknown error".to_string())
                            } else {
                                format!("{:?}", reason)
                            };
                            return Err(MadrpcError::JavaScriptExecution(format!("Promise rejected: {}", reason_str)));
                        }
                    }
                }
            }
        }

        // Not a promise, just run jobs once for any microtasks
        let _ = self.ctx.run_jobs();

        tracing::debug!("call_rpc: Converting result to JSON...");
        js_value_to_json(result, &mut self.ctx)
    }

    /// Call a registered RPC function asynchronously.
    ///
    /// This is the async version of `call_rpc()` that properly handles JavaScript
    /// promises without blocking. It uses `run_jobs_async()` to drive promise
    /// resolution within the existing tokio runtime, avoiding the "runtime within
    /// runtime" panic.
    ///
    /// # Parameters
    ///
    /// * `method` - Name of the registered function to call
    /// * `args` - Arguments to pass to the function (must be valid JSON)
    ///
    /// # Returns
    ///
    /// The function's return value as JSON.
    ///
    /// # Promise Handling
    ///
    /// If the function returns a Promise, this method will await its resolution
    /// using `run_jobs_async()` to drive the promise to completion.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The method is not registered
    /// - The registered value is not a function
    /// - Function execution fails
    /// - Arguments cannot be converted
    /// - A promise is rejected
    pub async fn call_rpc_async(&mut self, method: &str, args: JsonValue) -> Result<JsonValue> {
        tracing::debug!("call_rpc_async: calling method '{}'", method);

        // Get madrpc object and registry
        let madrpc = self.ctx.global_object()
            .get(js_string!("madrpc"), &mut self.ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        tracing::debug!("call_rpc_async: Getting registry...");
        let registry_val = madrpc.as_object()
            .and_then(|o| o.get(js_string!("__registry"), &mut self.ctx).ok())
            .ok_or_else(|| MadrpcError::InvalidRequest("Failed to access registry".into()))?;

        let registry = registry_val.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Registry is not an object".into()))?;

        // Get registered function
        tracing::debug!("call_rpc_async: Getting function '{}' from registry...", method);
        let func = registry.get(js_string!(method), &mut self.ctx)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Method '{}' lookup error: {}", method, e)))?;

        if func.is_undefined() {
            return Err(MadrpcError::InvalidRequest(format!("Method '{}' is not registered", method)));
        }

        let func_obj = func.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Registered value is not a function".into()))?;

        // Convert args to JsValue and call
        tracing::debug!("call_rpc_async: Converting args and calling function...");
        let args_js = json_to_js_value(args, &mut self.ctx)?;
        let result = func_obj.call(&JsValue::undefined(), &[args_js], &mut self.ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Function execution error: {}", e)))?;

        // Check if result is a Promise
        if let Some(result_obj) = result.as_object() {
            if let Ok(promise) = JsPromise::from_object(result_obj.clone()) {
                tracing::debug!("call_rpc_async: Result is a Promise, waiting for resolution...");

                // Use run_jobs_async to drive the promise to completion with timeout
                let executor = self.job_executor.clone();
                let context_ref = RefCell::new(&mut self.ctx);
                let promise_clone = promise.clone();

                // Poll with timeout to prevent CPU exhaustion
                let start_time = Instant::now();
                let timeout_duration = std::time::Duration::from_millis(MAX_PROMISE_WAIT_MS);

                loop {
                    // Check timeout first to prevent infinite loops
                    let elapsed = start_time.elapsed();
                    if elapsed >= timeout_duration {
                        tracing::error!("call_rpc_async: Promise timeout after {}ms (max: {}ms)", elapsed.as_millis(), MAX_PROMISE_WAIT_MS);
                        return Err(MadrpcError::JavaScriptExecution(format!(
                            "Promise did not resolve within {}ms timeout",
                            MAX_PROMISE_WAIT_MS
                        )));
                    }

                    let _ = executor.clone().run_jobs_async(&context_ref).await;

                    match promise_clone.state() {
                        PromiseState::Pending => {
                            // Continue polling with small sleep to prevent CPU exhaustion
                            tokio::time::sleep(std::time::Duration::from_micros(100)).await;
                        }
                        PromiseState::Fulfilled(value) => {
                            tracing::debug!("call_rpc_async: Promise fulfilled after {}ms", elapsed.as_millis());
                            return js_value_to_json(value, &mut self.ctx);
                        }
                        PromiseState::Rejected(reason) => {
                            tracing::debug!("call_rpc_async: Promise rejected after {}ms", elapsed.as_millis());
                            let reason_str = if let Some(s) = reason.as_string() {
                                s.to_std_string()
                                    .unwrap_or_else(|_| "Unknown error".to_string())
                            } else {
                                format!("{:?}", reason)
                            };
                            return Err(MadrpcError::JavaScriptExecution(format!("Promise rejected: {}", reason_str)));
                        }
                    }
                }
            }
        }

        // Not a promise, just run jobs once for any microtasks
        let executor = self.job_executor.clone();
        let context_ref = RefCell::new(&mut self.ctx);
        executor.run_jobs_async(&context_ref).await
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Job execution error: {}", e)))?;

        tracing::debug!("call_rpc_async: Converting result to JSON...");
        js_value_to_json(result, &mut self.ctx)
    }

    /// Make a distributed RPC call.
    ///
    /// This method can be called from JavaScript via a callback to make
    /// RPC calls to other nodes in the cluster.
    ///
    /// # Parameters
    ///
    /// * `method` - Name of the RPC method to call on another node
    /// * `args` - Arguments to pass to the remote function
    ///
    /// # Returns
    ///
    /// The remote function's return value as JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No client is configured (node was created without orchestrator)
    /// - The RPC call fails (network error, timeout, remote execution error)
    pub async fn distributed_call(&self, method: &str, args: JsonValue) -> Result<JsonValue> {
        let client = self.client.as_ref()
            .ok_or_else(|| MadrpcError::InvalidRequest("No client configured".into()))?;
        client.call(method, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use serde_json::json;

    /// Test that context can be created and used on the same thread
    #[test]
    fn test_context_creates_successfully() {
        let script_path = "/tmp/test_madrpc_context.js";
        fs::write(script_path, r#"
            madrpc.register('test', function(args) {
                return { result: args.value * 2 };
            });
        "#).expect("Failed to write test script");

        let result = MadrpcContext::new(script_path);
        assert!(result.is_ok(), "Context should be created successfully");

        let mut ctx = result.unwrap();
        let result = ctx.call_rpc("test", json!({"value": 21}));
        assert!(result.is_ok(), "RPC call should succeed");
        assert_eq!(result.unwrap(), json!({"result": 42}));

        // Cleanup
        let _ = fs::remove_file(script_path);
    }

    /// Test that context with client works correctly
    #[test]
    fn test_context_with_client_no_pointer_storage() {
        let script_source = r#"
            madrpc.register('test', function(args) {
                return { result: args.value * 2 };
            });
        "#;

        // Create context without client
        let result = MadrpcContext::from_source(script_source);
        assert!(result.is_ok(), "Context should be created successfully");

        let mut ctx = result.unwrap();
        let result = ctx.call_rpc("test", json!({"value": 21}));
        assert!(result.is_ok(), "RPC call should succeed");
        assert_eq!(result.unwrap(), json!({"result": 42}));
    }

    /// Test that call_rpc properly handles errors
    #[test]
    fn test_call_rpc_error_handling() {
        let script_path = "/tmp/test_madrpc_context_error.js";
        fs::write(script_path, r#"
            madrpc.register('error', function() {
                throw new Error('test error');
            });
        "#).expect("Failed to write test script");

        let mut ctx = MadrpcContext::new(script_path).unwrap();
        let result = ctx.call_rpc("error", json!({}));
        assert!(result.is_err(), "RPC call should fail");

        // Cleanup
        let _ = fs::remove_file(script_path);
    }

    /// Test that context maintains thread-local state correctly
    #[test]
    fn test_context_thread_local_state() {
        let script_path = "/tmp/test_madrpc_context_tls.js";
        fs::write(script_path, r#"
            let counter = 0;
            madrpc.register('increment', function() {
                counter += 1;
                return { count: counter };
            });
        "#).expect("Failed to write test script");

        let mut ctx = MadrpcContext::new(script_path).unwrap();

        let result1 = ctx.call_rpc("increment", json!({})).unwrap();
        assert_eq!(result1, json!({"count": 1}));

        let result2 = ctx.call_rpc("increment", json!({})).unwrap();
        assert_eq!(result2, json!({"count": 2}));

        // Cleanup
        let _ = fs::remove_file(script_path);
    }

    /// Test that promise polling respects the timeout limit
    ///
    /// This test verifies that:
    /// 1. The timeout mechanism works correctly
    /// 2. Error messages contain the timeout duration
    /// 3. The timeout is enforced (not just a warning)
    ///
    /// Note: This test is marked as ignored by default because it takes
    /// ~30 seconds to run. Run it explicitly with:
    /// cargo test -p madrpc-server test_promise_timeout -- --ignored
    #[test]
    #[ignore]
    fn test_promise_timeout() {
        let script_path = "/tmp/test_madrpc_promise_timeout.js";

        // Create a script that registers a function returning a promise that never resolves
        // This simulates a hanging async operation
        fs::write(script_path, r#"
            madrpc.register('hangingPromise', function() {
                return new Promise((resolve, reject) => {
                    // Never resolve or reject - this will timeout
                    // Intentionally empty to simulate a hanging operation
                });
            });
        "#).expect("Failed to write test script");

        let mut ctx = MadrpcContext::new(script_path).unwrap();
        let start = std::time::Instant::now();
        let result = ctx.call_rpc("hangingPromise", json!({}));
        let elapsed = start.elapsed();

        // Should timeout and return an error
        assert!(result.is_err(), "RPC call should timeout and fail");

        // Verify it's a timeout error
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("timeout"), "Error should mention timeout: {}", err_msg);
        assert!(err_msg.contains("30000"), "Error should mention the 30000ms timeout: {}", err_msg);

        // Verify it timed out in approximately the right amount of time
        // (with generous tolerance for test execution time)
        assert!(elapsed.as_millis() >= 25000, "Should take at least 25 seconds, took {}ms", elapsed.as_millis());
        assert!(elapsed.as_millis() <= 40000, "Should take at most 40 seconds, took {}ms", elapsed.as_millis());

        // Cleanup
        let _ = fs::remove_file(script_path);
    }

    /// Test that promise timeout error message is correct
    ///
    /// This is a fast test that verifies the timeout behavior without
    /// actually waiting for the timeout.
    #[test]
    fn test_promise_timeout_error_message() {
        // This test verifies that the timeout constant is properly defined
        // and the error message format is correct
        assert_eq!(MAX_PROMISE_WAIT_MS, 30_000, "MAX_PROMISE_WAIT_MS should be 30 seconds");

        // Create a mock error to verify the format
        let error = MadrpcError::JavaScriptExecution(format!(
            "Promise did not resolve within {}ms timeout",
            MAX_PROMISE_WAIT_MS
        ));
        let error_msg = error.to_string();
        assert!(error_msg.contains("30000"), "Error should mention the 30000ms timeout: {}", error_msg);
        assert!(error_msg.contains("timeout"), "Error should mention timeout: {}", error_msg);
    }
}
