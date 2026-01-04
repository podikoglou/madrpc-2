use boa_engine::{Context, js_string, native_function::NativeFunction, value::JsValue, object::{JsObject, FunctionObjectBuilder}};
use madrpc_common::protocol::error::{Result, MadrpcError};

/// Install all MaDRPC-specific bindings into the Boa context.
/// This is the SINGLE place where we expose custom functions to the JavaScript VM.
pub fn install_madrpc_bindings(ctx: &mut Context) -> Result<()> {
    // Create madrpc global object
    let madrpc_object = JsObject::default();

    // Create private storage for registered functions
    let registry = JsObject::default();
    madrpc_object.set(js_string!("__registry"), registry.clone(), false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

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

            let registry_obj = madrpc.as_object()
                .and_then(|o| o.get(js_string!("__registry"), context).ok())
                .and_then(|v| v.as_object().cloned())
                .ok_or_else(|| boa_engine::JsNativeError::typ()
                    .with_message("Registry not found"))?;

            registry_obj.set(name.clone(), func.clone(), true, context)
                .map_err(|e| boa_engine::JsNativeError::typ()
                    .with_message(format!("Failed to register: {}", e)))?;

            Ok(JsValue::Undefined)
        }),
    ).build();

    madrpc_object.set(js_string!("register"), register_fn, false, ctx)
        .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    // Register madrpc globally
    ctx.register_global_property(
        js_string!("madrpc"),
        madrpc_object,
        boa_engine::property::Attribute::all()
    ).map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

    Ok(())
}
