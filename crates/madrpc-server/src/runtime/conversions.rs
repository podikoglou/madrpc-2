use boa_engine::{Context, value::JsValue, js_string, Source};
use serde_json::Value as JsonValue;
use madrpc_common::protocol::error::{Result, MadrpcError};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for generating unique temporary variable names
///
/// This counter ensures that each call to `js_value_to_json` gets a unique
/// variable name, preventing any potential collisions in the global scope.
///
/// # Thread Safety
///
/// `AtomicU64` ensures thread-safe increments without requiring a mutex.
/// The ordering is `Relaxed` because we only need uniqueness, not any
/// specific ordering relative to other memory operations.
static TEMP_VAR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Convert serde_json::Value to Boa JsValue
pub fn json_to_js_value(json: JsonValue, ctx: &mut Context) -> Result<JsValue> {
    match json {
        JsonValue::Null => Ok(JsValue::null()),
        JsonValue::Bool(b) => Ok(JsValue::new(b)),
        JsonValue::Number(n) => {
            n.as_f64()
                .map(JsValue::new)
                .or_else(|| n.as_i64().map(|i| JsValue::new(i)))
                .ok_or_else(|| MadrpcError::InvalidRequest("Number out of range".into()))
        }
        JsonValue::String(s) => Ok(JsValue::new(js_string!(s))),
        JsonValue::Array(arr) => {
            // Create array by evaluating JavaScript code
            let array_code = format!("[{}]", arr.iter()
                .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()))
                .collect::<Vec<_>>()
                .join(","));
            let array_val = ctx.eval(Source::from_bytes(&array_code))
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to create array: {}", e)))?;
            Ok(array_val)
        }
        JsonValue::Object(obj) => {
            // Use JSON.parse via eval for objects
            let json_str = serde_json::to_string(&obj)
                .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to serialize object: {}", e)))?;

            // Escape the JSON string for JavaScript
            let escaped_json = json_str.replace('\\', "\\\\").replace('"', "\\\"");

            // Use eval to call JSON.parse
            let code = format!("JSON.parse(\"{}\")", escaped_json);
            let js_value = ctx.eval(Source::from_bytes(&code))
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("JSON.parse failed: {}", e)))?;

            Ok(js_value)
        }
    }
}

/// Convert Boa JsValue to serde_json::Value
pub fn js_value_to_json(value: JsValue, ctx: &mut Context) -> Result<JsonValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(JsonValue::Null);
    }

    if let Some(b) = value.as_boolean() {
        return Ok(JsonValue::Bool(b));
    }

    if let Some(n) = value.as_number() {
        return serde_json::Number::from_f64(n)
            .map(JsonValue::Number)
            .ok_or_else(|| MadrpcError::InvalidRequest("Invalid float".into()));
    }

    if let Some(s) = value.as_string() {
        return Ok(JsonValue::String(s.to_std_string().map_err(|e| {
            MadrpcError::InvalidRequest(format!("String conversion error: {:?}", e))
        })?));
    }

    if value.is_object() {
        // =====================================================================
        // Global Variable Approach for JSON Stringification
        // =====================================================================
        //
        // We use a global temporary variable to work around Boa's limitations
        // with directly passing JsValue objects to JSON.stringify via eval.
        //
        // ## Why This Approach?
        //
        // 1. Boa's eval() doesn't support passing JsValue references directly
        // 2. We need to store the object somewhere accessible to JavaScript code
        // 3. JSON.stringify must be called from JavaScript to get proper serialization
        //
        // ## Safety Guarantees
        //
        // 1. **Unique Variable Names**: Each call gets a unique ID via:
        //    - Process ID (prevents cross-process collisions)
        //    - Atomic counter (prevents same-process collisions)
        //    - Format: `_madrpc_str_{process_id}_{counter}`
        //
        // 2. **Proper Cleanup**: The temporary variable is explicitly deleted
        //    after use to prevent global scope pollution
        //
        // 3. **Thread Safety**: AtomicU64 ensures unique IDs across threads
        //    without requiring locks
        //
        // 4. **Isolation**: Each call uses a fresh variable name, preventing
        //    any interference between concurrent operations

        // Generate unique temporary variable name
        let unique_id = TEMP_VAR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_var = format!("_madrpc_str_{}_{}", std::process::id(), unique_id);

        // Store the value in a global variable
        ctx.global_object()
            .set(js_string!(temp_var.as_str()), value.clone(), true, ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to set temp variable: {}", e)))?;

        // Eval JSON.stringify on the global variable
        let code = format!("JSON.stringify({})", temp_var);
        let json_str = ctx.eval(Source::from_bytes(&code))
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("JSON.stringify failed: {}", e)))?;

        let json_string = json_str.as_string()
            .ok_or_else(|| MadrpcError::InvalidRequest("JSON.stringify didn't return string".into()))?
            .to_std_string()
            .map_err(|e| MadrpcError::InvalidRequest(format!("String conversion error: {:?}", e)))?;

        let parsed: JsonValue = serde_json::from_str(&json_string)
            .map_err(|e| MadrpcError::InvalidRequest(format!("JSON parse error: {}", e)))?;

        // Clean up the temporary variable from global scope
        let cleanup_code = format!("delete globalThis.{}", temp_var);
        ctx.eval(Source::from_bytes(&cleanup_code))
            .map_err(|e| {
                tracing::warn!("Failed to clean up temp variable '{}': {}", temp_var, e);
                MadrpcError::JavaScriptExecution(format!("Cleanup failed: {}", e))
            })?;

        return Ok(parsed);
    }

    if value.is_symbol() {
        return Ok(JsonValue::Null);
    }

    Ok(JsonValue::Null)
}
