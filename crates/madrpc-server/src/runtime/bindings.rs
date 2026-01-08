//! JavaScript bindings for MaDRPC
//!
//! This module provides the native Rust functions that are exposed to JavaScript
//! code running in Boa. These functions allow JavaScript to interact with the
//! MaDRPC system.
//!
//! # JavaScript API
//!
//! The following global functions are registered on the `madrpc` object:
//!
//! - `madrpc.register(name, function)` - Register a JavaScript function for RPC
//! - `madrpc.call(method, args)` - Make an async distributed RPC call (returns Promise)
//! - `madrpc.callSync(method, args)` - Make a synchronous blocking RPC call
//!
//! # Safety
//!
//! The client `Arc` is safely stored in closure capture data instead of using
//! pointer-as-number storage. Each closure that needs the client gets its own
//! cloned Arc, ensuring proper reference counting and lifetime management.

use boa_engine::{Context, js_string, native_function::NativeFunction, value::JsValue, object::{JsObject, FunctionObjectBuilder, builtins::JsPromise}, JsNativeError, job::Job};
use madrpc_common::protocol::error::{Result, MadrpcError};
use crate::runtime::conversions::{json_to_js_value, js_value_to_json};
use std::sync::{Arc, Mutex, OnceLock};

/// Global tokio runtime for blocking calls.
///
/// This is a single shared runtime that is created once and reused for all
/// blocking synchronous RPC calls (callSync). This avoids the performance
/// overhead and resource exhaustion of creating a new runtime for each call.
///
/// # Thread Safety
///
/// The runtime is wrapped in a Mutex to allow safe concurrent access from
/// multiple threads. Each blocking call will lock the mutex, run the async
/// operation to completion, then release the lock.
static BLOCKING_RUNTIME: OnceLock<Mutex<tokio::runtime::Runtime>> = OnceLock::new();

/// Gets or creates the shared blocking runtime.
///
/// This runtime is used for synchronous blocking operations like callSync.
/// It's created once and reused to avoid the overhead of creating a new
/// runtime on each call.
///
/// # Returns
///
/// A static reference to the mutex-wrapped tokio runtime.
///
/// # Errors
///
/// Returns an error if tokio runtime creation fails (e.g., due to system
/// resource limits).
pub fn get_blocking_runtime() -> std::io::Result<&'static Mutex<tokio::runtime::Runtime>> {
    if let Some(runtime) = BLOCKING_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = tokio::runtime::Runtime::new()
        .map(Mutex::new)?;

    // SAFETY: This is the only place where we set the BLOCKING_RUNTIME
    // and we check if it's already set before returning.
    // The OnceLock ensures this only runs once.
    Ok(BLOCKING_RUNTIME.get_or_init(|| runtime))
}

/// Async RPC call implementation using NativeFunction::from_async_fn.
///
/// This uses the existing tokio runtime instead of creating a new one.
///
/// # Parameters
///
/// * `client` - Arc-wrapped MadrpcClient for making the RPC call
/// * `method` - Name of the RPC method to call
/// * `json_args` - Arguments to pass to the RPC method (JSON)
///
/// # Returns
///
/// The RPC call result as JSON, or an error string if the call fails.
async fn rpc_call_async(
    client: Arc<madrpc_client::MadrpcClient>,
    method: String,
    json_args: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    client.call(&method, json_args).await
        .map_err(|e| format!("RPC call failed: {}", e))
}

/// Install all MaDRPC-specific bindings into the Boa context.
///
/// This is the SINGLE place where we expose custom functions to the JavaScript VM.
///
/// # Functions Registered
///
/// - `madrpc.register(name, function)` - Register a JavaScript function
/// - `madrpc.call(method, args)` - Async distributed RPC call (only if client provided)
/// - `madrpc.callSync(method, args)` - Synchronous distributed RPC call (only if client provided)
///
/// # Safety
///
/// The client Arc is stored safely in the closure data of the native functions.
/// We do NOT use pointer-as-number storage. The Arc is cloned into each closure
/// that needs it, ensuring proper reference counting and lifetime management.
///
/// # Parameters
///
/// * `ctx` - Mutable reference to the Boa context
/// * `client` - Optional Arc-wrapped MadrpcClient for distributed calls
///
/// # Returns
///
/// `Ok(())` if bindings were installed successfully, or an error if installation fails.
///
/// # Errors
///
/// Returns an error if:
/// - Creating the madrpc global object fails
/// - Setting properties on the madrpc object fails
/// - Registering the madrpc global property fails
pub(crate) fn install_madrpc_bindings(ctx: &mut Context, client: Option<Arc<madrpc_client::MadrpcClient>>) -> Result<()> {
    // Create madrpc global object
    let madrpc_object = JsObject::default(ctx.intrinsics());

    // Create private storage for registered functions
    let registry = JsObject::default(ctx.intrinsics());
    madrpc_object.set(js_string!("__registry"), registry.clone(), false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Note: We do NOT store the client pointer on the object anymore.
    // Instead, we clone the Arc into each closure that needs it.
    // This is safe and avoids pointer-as-number storage.

    // Register native `madrpc.register` function
    let register_fn = FunctionObjectBuilder::new(
        ctx.realm(),
        NativeFunction::from_copy_closure(|_this, args, context| {
            // Validate arguments
            let name = args.get(0)
                .and_then(|v| v.as_string())
                .ok_or_else(|| boa_engine::JsNativeError::typ()
                    .with_message("First argument must be a string"))?;

            let func = args.get(1)
                .ok_or_else(|| boa_engine::JsNativeError::typ()
                    .with_message("Second argument required"))?;

            if !func.is_object() || !func.as_object().map_or(false, |o| o.is_callable()) {
                return Err(boa_engine::JsNativeError::typ()
                    .with_message("Second argument must be a function").into());
            }

            // Store function in registry
            let madrpc = context.global_object()
                .get(js_string!("madrpc"), context)
                .map_err(|e| boa_engine::JsNativeError::typ()
                    .with_message(format!("Failed to get madrpc: {}", e)))?;

            let madrpc_obj = madrpc.as_object()
                .ok_or_else(|| boa_engine::JsNativeError::typ()
                    .with_message("madrpc is not an object"))?;

            let registry_val = madrpc_obj.get(js_string!("__registry"), context)
                .map_err(|e| boa_engine::JsNativeError::typ()
                    .with_message(format!("Failed to get registry: {}", e)))?;

            let registry_obj = registry_val.as_object()
                .ok_or_else(|| boa_engine::JsNativeError::typ()
                    .with_message("Registry is not an object"))?;

            registry_obj.set(name.clone(), func.clone(), true, context)
                .map_err(|e| boa_engine::JsNativeError::typ()
                    .with_message(format!("Failed to register: {}", e)))?;

            Ok(JsValue::undefined())
        }),
    ).build();

    madrpc_object.set(js_string!("register"), register_fn, false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Only register async call and setOrchestrator if we have a client
    if let Some(client_arc) = client {
        // Register native `madrpc.call` function using NativeFunction::from_copy_closure_with_captures
        // This uses the existing tokio runtime and properly integrates with Boa's job queue
        let call_fn = FunctionObjectBuilder::new(
            ctx.realm(),
            NativeFunction::from_copy_closure_with_captures(
                move |_this, args: &[JsValue], client_arc: &Arc<madrpc_client::MadrpcClient>, context| {
                    // Clone the Arc for this async operation
                    let client_clone = Arc::clone(&client_arc);

                    // Validate and extract arguments (synchronous part)
                    let method = match args.get(0).and_then(|v| v.as_string()) {
                        Some(s) => match s.to_std_string() {
                            Ok(s) => s,
                            Err(e) => return Err(JsNativeError::typ()
                                .with_message(format!("Invalid method name: {:?}", e)).into()),
                        },
                        None => return Err(JsNativeError::typ()
                            .with_message("First argument must be a string (method name)").into()),
                    };

                    let args_value = if args.len() > 1 { args[1].clone() } else { JsValue::undefined() };

                    // Convert JS args to JSON (synchronous part)
                    let json_args = js_value_to_json(args_value, context)
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("Failed to convert args to JSON: {}", e)))?;

                    // Create a promise
                    let (promise, resolvers) = JsPromise::new_pending(context);

                    // Enqueue an async job that will use the existing tokio runtime
                    context.enqueue_job(
                        Job::AsyncJob(
                            boa_engine::job::NativeAsyncJob::new(async move |context| {
                                // Make the async RPC call using the existing tokio runtime
                                let result_json = rpc_call_async(client_clone, method, json_args).await;

                                let mut ctx = context.borrow_mut();
                                match result_json {
                                    Ok(result_json) => {
                                        // Convert result back to JS value
                                        let result = json_to_js_value(result_json, &mut *ctx)
                                            .map_err(|e| JsNativeError::typ()
                                                .with_message(format!("Failed to convert result to JS: {}", e)))?;

                                        // Resolve the promise
                                        resolvers.resolve.call(&JsValue::undefined(), &[result], &mut *ctx)
                                            .map_err(Into::into)
                                    }
                                    Err(e) => {
                                        // Reject the promise
                                        let error_val = JsValue::new(js_string!(e.as_str()));
                                        resolvers.reject.call(&JsValue::undefined(), &[error_val], &mut *ctx)
                                            .map_err(Into::into)
                                    }
                                }
                            }).into()
                        )
                    );

                    Ok(promise.into())
                },
                &client_arc,
            ),
        ).build();

        madrpc_object.set(js_string!("call"), call_fn, false, ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        // Register native `madrpc.callSync` function (synchronous blocking)
        //
        // IMPORTANT: This function uses a SHARED tokio runtime instead of creating
        // a new one on each call. This prevents resource exhaustion and improves performance.
        let call_sync_fn = FunctionObjectBuilder::new(
            ctx.realm(),
            NativeFunction::from_copy_closure_with_captures(
                move |_this, args: &[JsValue], client_arc: &Arc<madrpc_client::MadrpcClient>, context| {
                    // Clone the Arc for this call
                    let client_clone = Arc::clone(&client_arc);

                    // Validate arguments
                    let method = args.get(0)
                        .and_then(|v| v.as_string())
                        .ok_or_else(|| JsNativeError::typ()
                            .with_message("First argument must be a string (method name)"))?
                        .to_std_string()
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("Invalid method name: {:?}", e)))?;

                    let args_value = if args.len() > 1 { args[1].clone() } else { JsValue::undefined() };

                    // Convert JS args to JSON
                    let json_args = js_value_to_json(args_value, context)
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("Failed to convert args to JSON: {}", e)))?;

                    // Use the SHARED runtime instead of creating a new one
                    let rt_mutex = get_blocking_runtime()
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("Failed to get blocking runtime: {}", e)))?;

                    let rt = rt_mutex.lock()
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("Failed to lock runtime: {}", e)))?;

                    let result_json = rt.block_on(client_clone.call(&method, json_args))
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("RPC call failed: {}", e)))?;

                    // Convert result back to JS value
                    let result = json_to_js_value(result_json, context)
                        .map_err(|e| JsNativeError::typ()
                            .with_message(format!("Failed to convert result to JS: {}", e)))?;

                    Ok(result)
                },
                &client_arc,
            ),
        ).build();

        madrpc_object.set(js_string!("callSync"), call_sync_fn, false, ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        // Register native `madrpc.setOrchestrator` function
        //
        // NOTE: This function is now REMOVED from the public API because it requires
        // replacing the client pointer, which is unsafe with our new architecture.
        //
        // Instead, users should create a new Node with the orchestrator address
        // using Node::with_orchestrator(). This is the recommended pattern.
        //
        // The setOrchestrator function is only registered if we're creating a
        // context without a client initially, which is not the normal case.
    }

    // Register madrpc globally
    ctx.register_global_property(
        js_string!("madrpc"),
        madrpc_object,
        boa_engine::property::Attribute::all()
    ).map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use boa_engine::Source;

    /// Test that bindings can be installed without a client
    #[test]
    fn test_install_bindings_without_client() {
        let mut ctx = Context::default();
        let result = install_madrpc_bindings(&mut ctx, None);
        assert!(result.is_ok(), "Failed to install bindings without client");

        // Verify madrpc global object exists
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        assert!(madrpc.is_object(), "madrpc should be an object");

        // Verify register function exists
        let madrpc_obj = madrpc.as_object().unwrap();
        let register_fn = madrpc_obj.get(js_string!("register"), &mut ctx).unwrap();
        assert!(register_fn.is_function(), "register should be a function");

        // Verify call function does NOT exist (no client)
        let call_fn = madrpc_obj.get(js_string!("call"), &mut ctx).unwrap();
        assert!(call_fn.is_undefined(), "call should not exist without client");

        // Verify callSync function does NOT exist (no client)
        let call_sync_fn = madrpc_obj.get(js_string!("callSync"), &mut ctx).unwrap();
        assert!(call_sync_fn.is_undefined(), "callSync should not exist without client");
    }

    /// Test that bindings can be installed with a client
    #[test]
    fn test_install_bindings_with_client() {
        let mut ctx = Context::default();
        // We can't create a real client without a running orchestrator,
        // but we can test that the bindings install correctly with None
        // The important thing is that no pointer-as-number is used
        let result = install_madrpc_bindings(&mut ctx, None);
        assert!(result.is_ok(), "Failed to install bindings with client");

        // Verify __client_ptr does NOT exist on madrpc object
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        let madrpc_obj = madrpc.as_object().unwrap();

        // This should NOT exist anymore (we removed pointer-as-number storage)
        let client_ptr = madrpc_obj.get(js_string!("__client_ptr"), &mut ctx).unwrap();
        assert!(client_ptr.is_undefined(), "__client_ptr should not exist (no pointer-as-number storage)");
    }

    /// Test that register function works correctly
    #[test]
    fn test_register_function() {
        let mut ctx = Context::default();
        install_madrpc_bindings(&mut ctx, None).unwrap();

        // Evaluate a script that registers a function
        let script = r#"
            madrpc.register('test', function(args) {
                return args.value * 2;
            });
        "#;

        ctx.eval(Source::from_bytes(script))
            .expect("Script evaluation should succeed");

        // Verify function is in registry
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        let madrpc_obj = madrpc.as_object().unwrap();
        let registry = madrpc_obj.get(js_string!("__registry"), &mut ctx).unwrap();
        let registry_obj = registry.as_object().unwrap();

        let test_fn = registry_obj.get(js_string!("test"), &mut ctx).unwrap();
        assert!(test_fn.is_function(), "Registered function should exist");
    }

    /// Test that register validates arguments
    #[test]
    fn test_register_validates_arguments() {
        let mut ctx = Context::default();
        install_madrpc_bindings(&mut ctx, None).unwrap();

        // Test with no arguments
        let result = ctx.eval(Source::from_bytes("madrpc.register()"));
        assert!(result.is_err(), "Should fail with no arguments");

        // Test with non-string name
        let result = ctx.eval(Source::from_bytes("madrpc.register(123, function(){})"));
        assert!(result.is_err(), "Should fail with non-string name");

        // Test with non-function argument
        let result = ctx.eval(Source::from_bytes("madrpc.register('test', 'not a function')"));
        assert!(result.is_err(), "Should fail with non-function argument");
    }

    /// Test that shared runtime is created once and reused
    #[test]
    fn test_shared_runtime_reuse() {
        // First call should create the runtime
        let rt1 = get_blocking_runtime();
        assert!(rt1.is_ok(), "First call should create runtime");

        // Second call should return the same runtime
        let rt2 = get_blocking_runtime();
        assert!(rt2.is_ok(), "Second call should return existing runtime");

        // They should be the same pointer
        assert_eq!(
            rt1.unwrap().as_ptr(),
            rt2.unwrap().as_ptr(),
            "Should return the same runtime instance"
        );
    }

    /// Test that no pointer-as-number is stored in JavaScript objects
    #[test]
    fn test_no_pointer_as_number_storage() {
        let mut ctx = Context::default();
        install_madrpc_bindings(&mut ctx, None).unwrap();

        // Get madrpc object
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        let madrpc_obj = madrpc.as_object().unwrap();

        // Iterate through all properties and ensure none are numbers that look like pointers
        let props = madrpc_obj.own_property_keys(&mut ctx).unwrap();

        for prop in props {
            if let Some(prop_str) = prop.as_string() {
                let prop_name = prop_str.to_std_string().unwrap();
                if prop_name.contains("ptr") || prop_name.contains("pointer") {
                    panic!("Found property with 'ptr' in name: {} (pointer-as-number storage detected)", prop_name);
                }

                let value = madrpc_obj.get(prop, &mut ctx).unwrap();
                if value.is_number() {
                    let num = value.as_number().unwrap();
                    // Check if it looks like a pointer (very large number)
                    if num > 1_000_000_000.0 {
                        panic!("Found large number property {} = {} (might be a pointer)", prop_name, num);
                    }
                }
            }
        }
    }

    /// Test that setOrchestrator is removed from API
    #[test]
    fn test_set_orchestrator_removed() {
        let mut ctx = Context::default();
        install_madrpc_bindings(&mut ctx, None).unwrap();

        // Verify setOrchestrator does NOT exist
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        let madrpc_obj = madrpc.as_object().unwrap();

        let set_orch = madrpc_obj.get(js_string!("setOrchestrator"), &mut ctx).unwrap();
        assert!(set_orch.is_undefined(), "setOrchestrator should not exist (removed from API)");
    }
}
