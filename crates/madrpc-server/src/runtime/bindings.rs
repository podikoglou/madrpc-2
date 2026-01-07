use boa_engine::{Context, js_string, native_function::NativeFunction, value::JsValue, object::{JsObject, FunctionObjectBuilder, builtins::JsPromise}, JsNativeError, JsResult, job::NativeAsyncJob};
use madrpc_common::protocol::error::{Result, MadrpcError};
use crate::runtime::conversions::{json_to_js_value, js_value_to_json};
use std::sync::Arc;
use std::cell::RefCell;

/// Async helper for madrpc.call
async fn call_async_impl(
    method: String,
    args_value: JsValue,
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let mut ctx = context.borrow_mut();

    // Get client pointer from madrpc object
    let madrpc = ctx.global_object()
        .get(js_string!("madrpc"), &mut *ctx)
        .map_err(|e| JsNativeError::typ()
            .with_message(format!("Failed to get madrpc: {}", e)))?;

    let madrpc_obj = madrpc.as_object()
        .ok_or_else(|| JsNativeError::typ()
            .with_message("madrpc is not an object"))?;

    let client_ptr_val = madrpc_obj.get(js_string!("__client_ptr"), &mut *ctx)
        .map_err(|e| JsNativeError::typ()
            .with_message(format!("Failed to get __client_ptr: {}", e)))?;

    let client_ptr_usize = client_ptr_val.as_number()
        .ok_or_else(|| JsNativeError::typ()
            .with_message("__client_ptr is not a number"))?
        as usize;

    // Safety: The client was stored as an Arc pointer by install_madrpc_bindings
    // We recreate the Arc here and increment the refcount
    let client_arc: Arc<madrpc_client::MadrpcClient> = unsafe { Arc::from_raw(client_ptr_usize as *const madrpc_client::MadrpcClient) };
    let client_clone = client_arc.clone();
    // Leak the original Arc again so it remains valid for future calls
    let _ = Arc::into_raw(client_arc);

    // Convert JS args to JSON
    let json_args = js_value_to_json(args_value, &mut *ctx)
        .map_err(|e| JsNativeError::typ()
            .with_message(format!("Failed to convert args to JSON: {}", e)))?;

    // Drop context before async call
    drop(ctx);

    // Make the async RPC call
    let result_json = client_clone.call(&method, json_args).await
        .map_err(|e| JsNativeError::typ()
            .with_message(format!("RPC call failed: {}", e)))?;

    // Re-borrow context to convert result back to JS
    let mut ctx = context.borrow_mut();

    // Convert result back to JS value
    let result = json_to_js_value(result_json, &mut *ctx)
        .map_err(|e| JsNativeError::typ()
            .with_message(format!("Failed to convert result to JS: {}", e)))?;

    Ok(result)
}

/// Install all MaDRPC-specific bindings into the Boa context.
/// This is the SINGLE place where we expose custom functions to the JavaScript VM.
pub fn install_madrpc_bindings(ctx: &mut Context, client: Option<Arc<madrpc_client::MadrpcClient>>) -> Result<()> {
    // Create madrpc global object
    let madrpc_object = JsObject::default(ctx.intrinsics());

    // Create private storage for registered functions
    let registry = JsObject::default(ctx.intrinsics());
    madrpc_object.set(js_string!("__registry"), registry.clone(), false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Store client reference if provided (as a private pointer property)
    if let Some(client_ref) = client {
        let client_ptr = Arc::into_raw(client_ref) as *const madrpc_client::MadrpcClient as usize;
        madrpc_object.set(js_string!("__client_ptr"), JsValue::new(client_ptr as f64), false, ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;
    }

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

    // Register native `madrpc.call` function (async)
    let call_fn = FunctionObjectBuilder::new(
        ctx.realm(),
        NativeFunction::from_copy_closure(move |_this, args, context| {
            // Validate and extract arguments
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

            // Create a promise
            let (promise, resolvers) = JsPromise::new_pending(context);

            // Enqueue the async job
            context.enqueue_job(
                NativeAsyncJob::new(async move |ctx| {
                    let result = call_async_impl(method, args_value, ctx).await;
                    let context = &mut ctx.borrow_mut();
                    match result {
                        Ok(v) => {
                            resolvers.resolve.call(&JsValue::undefined(), &[v], context)
                                .map_err(Into::into)
                        }
                        Err(e) => {
                            let e = e.into_opaque(context)?;
                            resolvers.reject.call(&JsValue::undefined(), &[e], context)
                                .map_err(Into::into)
                        }
                    }
                }).into()
            );

            Ok(promise.into())
        }),
    ).build();

    madrpc_object.set(js_string!("call"), call_fn, false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Register native `madrpc.callSync` function (synchronous blocking)
    let call_sync_fn = FunctionObjectBuilder::new(
        ctx.realm(),
        NativeFunction::from_copy_closure(|_this, args, context| {
            // Validate arguments
            let method = args.get(0)
                .and_then(|v| v.as_string())
                .ok_or_else(|| JsNativeError::typ()
                    .with_message("First argument must be a string (method name)"))?
                .to_std_string()
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Invalid method name: {:?}", e)))?;

            let args_value = if args.len() > 1 { args[1].clone() } else { JsValue::undefined() };

            // Get client pointer from madrpc object
            let madrpc = context.global_object()
                .get(js_string!("madrpc"), context)
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to get madrpc: {}", e)))?;

            let madrpc_obj = madrpc.as_object()
                .ok_or_else(|| JsNativeError::typ()
                    .with_message("madrpc is not an object"))?;

            let client_ptr_val = madrpc_obj.get(js_string!("__client_ptr"), context)
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to get __client_ptr: {}", e)))?;

            let client_ptr_usize = client_ptr_val.as_number()
                .ok_or_else(|| JsNativeError::typ()
                    .with_message("__client_ptr is not a number"))?
                as usize;

            // Safety: The client was stored as an Arc pointer by install_madrpc_bindings
            let client_arc: Arc<madrpc_client::MadrpcClient> = unsafe { Arc::from_raw(client_ptr_usize as *const madrpc_client::MadrpcClient) };
            let client_clone = client_arc.clone();
            let _ = Arc::into_raw(client_arc);

            // Convert JS args to JSON
            let json_args = js_value_to_json(args_value, context)
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to convert args to JSON: {}", e)))?;

            // Create a runtime and block on the async call
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to create tokio runtime: {}", e)))?;

            let result_json = rt.block_on(client_clone.call(&method, json_args))
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("RPC call failed: {}", e)))?;

            // Convert result back to JS value
            let result = json_to_js_value(result_json, context)
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to convert result to JS: {}", e)))?;

            Ok(result)
        }),
    ).build();

    madrpc_object.set(js_string!("callSync"), call_sync_fn, false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Register native `madrpc.setOrchestrator` function
    let set_orchestrator_fn = FunctionObjectBuilder::new(
        ctx.realm(),
        NativeFunction::from_copy_closure(|_this, args, context| {
            // Validate arguments
            let addr = args.get(0)
                .and_then(|v| v.as_string())
                .ok_or_else(|| JsNativeError::typ()
                    .with_message("First argument must be a string (orchestrator address)"))?
                .to_std_string()
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Invalid orchestrator address: {:?}", e)))?;

            // Create a runtime and block on client creation
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to create tokio runtime: {}", e)))?;

            let client = rt.block_on(madrpc_client::MadrpcClient::new(&addr))
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to create client: {}", e)))?;

            // Wrap in Arc and store pointer
            let client_arc = Arc::new(client);
            let client_ptr = Arc::into_raw(client_arc) as *const madrpc_client::MadrpcClient as usize;

            // Get madrpc object
            let madrpc = context.global_object()
                .get(js_string!("madrpc"), context)
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to get madrpc: {}", e)))?;

            let madrpc_obj = madrpc.as_object()
                .ok_or_else(|| JsNativeError::typ()
                    .with_message("madrpc is not an object"))?;

            // Update the client pointer
            madrpc_obj.set(js_string!("__client_ptr"), JsValue::new(client_ptr as f64), false, context)
                .map_err(|e| JsNativeError::typ()
                    .with_message(format!("Failed to set __client_ptr: {}", e)))?;

            Ok(JsValue::undefined())
        }),
    ).build();

    madrpc_object.set(js_string!("setOrchestrator"), set_orchestrator_fn, false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Register madrpc globally
    ctx.register_global_property(
        js_string!("madrpc"),
        madrpc_object,
        boa_engine::property::Attribute::all()
    ).map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    Ok(())
}
