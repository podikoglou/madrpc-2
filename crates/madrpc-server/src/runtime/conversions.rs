//! JSON <-> JavaScript Value Conversions
//!
//! This module provides bidirectional conversion between `serde_json::Value` and
//! Boa's `JsValue`. These conversions are essential for:
//!
//! - Passing RPC arguments from the wire format (JSON) to JavaScript functions
//! - Returning JavaScript function results as JSON for transmission
//!
//! # Type Mapping
//!
//! | JSON Type | JavaScript Type |
//! |-----------|-----------------|
//! | null | null |
//! | boolean | Boolean |
//! | number | Number |
//! | string | String |
//! | array | Array |
//! | object | Object |
//!
//! # Limitations
//!
//! - Symbol keys in JavaScript objects are skipped during conversion
//! - JavaScript symbols are converted to JSON null
//! - Numbers outside valid JSON ranges cause errors

use boa_engine::{
    Context,
    value::JsValue,
    js_string,
    object::{JsObject, builtins::JsArray},
    property::PropertyKey,
};
use serde_json::Value as JsonValue;
use madrpc_common::protocol::error::{Result, MadrpcError};

/// Convert serde_json::Value to Boa JsValue.
///
/// This function recursively converts JSON values to their JavaScript equivalents,
/// handling primitive types, arrays, and nested objects.
///
/// # Arguments
///
/// * `json` - The JSON value to convert
/// * `ctx` - Mutable reference to the Boa context (used for object creation)
///
/// # Returns
///
/// A `JsValue` representing the equivalent JavaScript value
///
/// # Errors
///
/// Returns `MadrpcError::InvalidRequest` if:
/// - A number is out of range for JavaScript's Number type
/// - Object property creation fails
/// - Array push operation fails
///
/// # Examples
///
/// ```ignore
/// let mut ctx = Context::default();
/// let json = json!({"name": "test", "value": 42});
/// let js_value = json_to_js_value(json, &mut ctx)?;
/// ```
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
            // Create array using native Boa API
            let js_array = JsArray::new(ctx);
            for (i, v) in arr.iter().enumerate() {
                let js_value = json_to_js_value(v.clone(), ctx)?;
                js_array.push(js_value, ctx)
                    .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to push array element {}: {}", i, e)))?;
            }
            Ok(js_array.into())
        }
        JsonValue::Object(obj) => {
            // Create object using native Boa API
            let js_obj = JsObject::with_object_proto(ctx.intrinsics());

            for (key, value) in obj {
                let js_value = json_to_js_value(value, ctx)?;
                js_obj.create_data_property_or_throw(
                    js_string!(key.clone()),
                    js_value,
                    ctx
                )
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to set property '{}': {}", key, e)))?;
            }

            Ok(js_obj.into())
        }
    }
}

/// Convert Boa JsValue to serde_json::Value.
///
/// This function recursively converts JavaScript values to their JSON equivalents,
/// handling primitives, arrays, objects, and special cases like undefined and symbols.
///
/// # Arguments
///
/// * `value` - The JavaScript value to convert
/// * `ctx` - Mutable reference to the Boa context (used for property access)
///
/// # Returns
///
/// A `serde_json::Value` representing the equivalent JSON value
///
/// # Conversion Rules
///
/// - `undefined` and `null` → JSON `null`
/// - `Boolean` → JSON `boolean`
/// - `Number` → JSON `number`
/// - `String` → JSON `string`
/// - `Array` → JSON `array` (recursively converts elements)
/// - `Object` → JSON `object` (skips symbol keys, recursively converts values)
/// - `Symbol` → JSON `null`
///
/// # Errors
///
/// Returns `MadrpcError::InvalidRequest` if:
/// - Array length overflows when converting to usize
/// - String conversion from UTF-16 fails
/// - Property access fails
/// - A float value cannot be represented as a JSON number
pub fn js_value_to_json(value: JsValue, ctx: &mut Context) -> Result<JsonValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(JsonValue::Null);
    }

    if let Some(b) = value.as_boolean() {
        return Ok(JsonValue::Bool(b));
    }

    if let Some(i) = value.as_i32() {
        return Ok(JsonValue::Number(i.into()));
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
        let obj = value.as_object()
            .ok_or_else(|| MadrpcError::InvalidRequest("Value is object but couldn't get object reference".into()))?;

        // Check if it's an array
        if obj.is_array() {
            let array = JsArray::from_object(obj.clone())
                .map_err(|e| MadrpcError::InvalidRequest(format!("Object is not a valid array: {}", e)))?;

            let length = array.length(ctx)
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to get array length: {}", e)))?
                .try_into()
                .map_err(|_| MadrpcError::InvalidRequest("Array length overflow".into()))?;

            let mut result = Vec::with_capacity(length);
            for i in 0..length {
                let elem = array.get(i, ctx)
                    .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to get array element {}: {}", i, e)))?;
                result.push(js_value_to_json(elem, ctx)?);
            }
            return Ok(JsonValue::Array(result));
        }

        // It's a plain object - iterate over its properties
        let keys = obj.own_property_keys(ctx)
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to get object keys: {}", e)))?;

        let mut result = serde_json::Map::new();

        for key in keys {
            // Convert PropertyKey to string
            let key_str = match &key {
                PropertyKey::String(s) => s.to_std_string()
                    .map_err(|e| MadrpcError::InvalidRequest(format!("String conversion error: {:?}", e))),
                PropertyKey::Index(i) => Ok(i.get().to_string()),
                PropertyKey::Symbol(_) => continue, // Skip symbol keys
            }?;

            let prop_value = obj.get(key.clone(), ctx)
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to get property '{}': {}", key_str, e)))?;
            result.insert(key_str, js_value_to_json(prop_value, ctx)?);
        }

        return Ok(JsonValue::Object(result));
    }

    if value.is_symbol() {
        return Ok(JsonValue::Null);
    }

    Ok(JsonValue::Null)
}
