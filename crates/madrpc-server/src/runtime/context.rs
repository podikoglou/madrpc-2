use boa_engine::{Context, Source, js_string, value::JsValue};
use std::path::Path;
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};

use crate::runtime::{bindings, conversions::{json_to_js_value, js_value_to_json}};

/// Boa context wrapper with MaDRPC bindings
///
/// The Context is wrapped in Mutex for thread safety. Boa's Context
/// is not thread-safe and must be accessed from a single thread.
pub struct MadrpcContext {
    ctx: Mutex<Context>,
    client: Option<Arc<madrpc_client::MadrpcClient>>,
}

// Safety: Boa Context is not thread-safe, wrapped in Mutex
unsafe impl Send for MadrpcContext {}
unsafe impl Sync for MadrpcContext {}


impl MadrpcContext {
    /// Create a new Boa context with MaDRPC bindings
    pub fn new(script_path: impl AsRef<Path>) -> Result<Self> {
        Self::with_client(script_path, None)
    }

    /// Create a new Boa context with optional client for distributed calls
    pub fn with_client(
        script_path: impl AsRef<Path>,
        client: Option<madrpc_client::MadrpcClient>,
    ) -> Result<Self> {
        let mut ctx = Context::default();
        let client = client.map(Arc::new);

        // Install madrpc bindings (native Rust functions)
        bindings::install_madrpc_bindings(&mut ctx)?;

        // Load and evaluate the script
        let script = std::fs::read_to_string(script_path)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to load script: {}", e)))?;

        ctx.eval(Source::from_bytes(&script))
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Script evaluation error: {}", e)))?;

        Ok(Self {
            ctx: Mutex::new(ctx),
            client,
        })
    }

    /// Call a registered RPC function
    pub fn call_rpc(&self, method: &str, args: JsonValue) -> Result<JsonValue> {
        tracing::debug!("call_rpc: Locking context mutex...");
        let mut ctx = self.ctx.lock().unwrap();
        tracing::debug!("call_rpc: Context locked");

        // Get madrpc object and registry
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut *ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        tracing::debug!("call_rpc: Getting registry...");
        let registry_val = madrpc.as_object()
            .and_then(|o| o.get(js_string!("__registry"), &mut *ctx).ok())
            .ok_or_else(|| MadrpcError::InvalidRequest("Failed to access registry".into()))?;

        let registry = registry_val.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Registry is not an object".into()))?;

        // Get registered function
        tracing::debug!("call_rpc: Getting function '{}' from registry...", method);
        let func = registry.get(js_string!(method), &mut *ctx)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Method '{}' lookup error: {}", method, e)))?;

        if func.is_undefined() {
            return Err(MadrpcError::InvalidRequest(format!("Method '{}' is not registered", method)));
        }

        let func_obj = func.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Registered value is not a function".into()))?;

        // Convert args to JsValue and call
        tracing::debug!("call_rpc: Converting args and calling function...");
        let args_js = json_to_js_value(args, &mut *ctx)?;
        let result = func_obj.call(&JsValue::undefined(), &[args_js], &mut *ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Function execution error: {}", e)))?;

        tracing::debug!("call_rpc: Converting result to JSON...");
        js_value_to_json(result, &mut *ctx)
    }

    /// Make a distributed RPC call (callable from JavaScript via a callback)
    pub async fn distributed_call(&self, method: &str, args: JsonValue) -> Result<JsonValue> {
        let client = self.client.as_ref()
            .ok_or_else(|| MadrpcError::InvalidRequest("No client configured".into()))?;
        client.call(method, args).await
    }
}
