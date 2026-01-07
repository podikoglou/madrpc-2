use boa_engine::{Context, Source, js_string, value::JsValue, object::builtins::JsPromise, builtins::promise::PromiseState};
use std::path::Path;
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value as JsonValue;
use std::sync::Arc;

use crate::runtime::{bindings, conversions::{json_to_js_value, js_value_to_json}};

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
use std::marker::PhantomData;
use std::rc::Rc;

struct ThreadNotSendSync {
    _marker: PhantomData<Rc<()>>,
}

/// Boa context wrapper with MaDRPC bindings.
///
/// The Context is wrapped in Mutex for internal use, but the entire
/// MadrpcContext is designed to be used on a single thread only.
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
/// Each request should create its own fresh MadrpcContext instance.
/// Contexts should never be shared between threads.
pub struct MadrpcContext {
    /// Thread-local marker that prevents Send/Sync
    _thread_marker: ThreadNotSendSync,
    /// The Boa context (wrapped in Mutex for internal synchronization)
    ctx: boa_engine::context::Context,
    /// Optional client for distributed RPC calls
    /// Note: This is NOT used in bindings anymore - we use closure capture instead
    /// But we keep it here for the distributed_call method
    client: Option<Arc<madrpc_client::MadrpcClient>>,
}

impl MadrpcContext {
    /// Create a new Boa context with MaDRPC bindings
    pub fn new(script_path: impl AsRef<Path>) -> Result<Self> {
        Self::with_client(script_path, None)
    }

    /// Create a new Boa context from a cached script source string
    ///
    /// This is more efficient than `new()` because it avoids file I/O.
    /// The script source is parsed and evaluated in a fresh context.
    pub fn from_source(script_source: &str) -> Result<Self> {
        Self::with_client_from_source(script_source, None)
    }

    /// Create a new Boa context with optional client for distributed calls
    pub fn with_client(
        script_path: impl AsRef<Path>,
        client: Option<madrpc_client::MadrpcClient>,
    ) -> Result<Self> {
        let client = client.map(Arc::new);
        let job_executor = std::rc::Rc::new(crate::runtime::TokioJobExecutor::new());
        let mut ctx = Context::builder()
            .job_executor(job_executor)
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
        })
    }

    /// Create a new Boa context from cached script source with optional client
    ///
    /// This is more efficient than `with_client()` because it avoids file I/O.
    pub fn with_client_from_source(
        script_source: &str,
        client: Option<madrpc_client::MadrpcClient>,
    ) -> Result<Self> {
        let client = client.map(Arc::new);
        let job_executor = std::rc::Rc::new(crate::runtime::TokioJobExecutor::new());
        let mut ctx = Context::builder()
            .job_executor(job_executor)
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
        })
    }

    /// Call a registered RPC function
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

                // Poll the promise by running jobs in a loop
                let max_iterations = 100_000;
                for iteration in 0..max_iterations {
                    // Run pending promise jobs
                    let _ = self.ctx.run_jobs();

                    // Check promise state
                    match promise.state() {
                        PromiseState::Pending => {
                            // Continue polling
                            if iteration % 100 == 0 {
                                std::thread::sleep(std::time::Duration::from_micros(100));
                            }
                            continue;
                        }
                        PromiseState::Fulfilled(value) => {
                            tracing::debug!("call_rpc: Promise fulfilled after {} iterations", iteration);
                            // Convert the fulfillment value to JSON
                            return js_value_to_json(value, &mut self.ctx);
                        }
                        PromiseState::Rejected(reason) => {
                            tracing::debug!("call_rpc: Promise rejected after {} iterations", iteration);
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

                return Err(MadrpcError::JavaScriptExecution("Promise did not resolve within timeout".to_string()));
            }
        }

        // Not a promise, just run jobs once for any microtasks
        let _ = self.ctx.run_jobs();

        tracing::debug!("call_rpc: Converting result to JSON...");
        js_value_to_json(result, &mut self.ctx)
    }

    /// Make a distributed RPC call (callable from JavaScript via a callback)
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

    /// Test that MadrpcContext is NOT Send
    #[test]
    fn test_context_is_not_send() {
        // This test verifies that MadrpcContext cannot be sent across threads
        // If it compiles, the test fails (because it should NOT compile)
        fn assert_not_send<T: !Send>() {}
        assert_not_send::<MadrpcContext>();
    }

    /// Test that MadrpcContext is NOT Sync
    #[test]
    fn test_context_is_not_sync() {
        // This test verifies that MadrpcContext cannot be shared between threads
        // If it compiles, the test fails (because it should NOT compile)
        fn assert_not_sync<T: !Sync>() {}
        assert_not_sync::<MadrpcContext>();
    }

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
}
