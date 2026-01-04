use boa_engine::{Context, value::JsValue, js_string, Source};
use serde_json::Value as JsonValue;
use madrpc_common::protocol::error::{Result, MadrpcError};

/// Convert serde_json::Value to Boa JsValue
pub fn json_to_js_value(json: JsonValue, ctx: &mut Context) -> Result<JsValue> {
    match json {
        JsonValue::Null => Ok(JsValue::Null),
        JsonValue::Bool(b) => Ok(JsValue::Boolean(b)),
        JsonValue::Number(n) => {
            n.as_f64()
                .map(JsValue::Rational)
                .or_else(|| n.as_i64().map(|i| JsValue::Integer(i as i32)))
                .ok_or_else(|| MadrpcError::InvalidRequest("Number out of range".into()))
        }
        JsonValue::String(s) => Ok(js_string!(s).into()),
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
            // Create object by evaluating JavaScript code
            let obj_code = format!("({{{}}}", obj.iter()
                .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string()),
                                       serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())))
                .collect::<Vec<_>>()
                .join(","));
            let obj_val = ctx.eval(Source::from_bytes(&obj_code))
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to create object: {}", e)))?;
            Ok(obj_val)
        }
    }
}

/// Convert Boa JsValue to serde_json::Value
pub fn js_value_to_json(value: JsValue, ctx: &mut Context) -> Result<JsonValue> {
    match value {
        JsValue::Undefined | JsValue::Null => Ok(JsonValue::Null),
        JsValue::Boolean(b) => Ok(JsonValue::Bool(b)),
        JsValue::Rational(f) => {
            serde_json::Number::from_f64(f)
                .map(JsonValue::Number)
                .ok_or_else(|| MadrpcError::InvalidRequest("Invalid float".into()))
        }
        JsValue::Integer(i) => Ok(JsonValue::Number(serde_json::Number::from(i))),
        JsValue::String(ref s) => Ok(JsonValue::String(s.to_std_string().map_err(|e| {
            MadrpcError::InvalidRequest(format!("String conversion error: {:?}", e))
        })?)),
        JsValue::Object(ref _obj) => {
            // Use JSON.stringify to convert the object/array to JSON string
            let json_obj = ctx.global_object()
                .get(js_string!("JSON"), ctx)
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("Failed to get JSON object: {}", e)))?;

            let stringify_fn = json_obj.as_object()
                .and_then(|o| o.get(js_string!("stringify"), ctx).ok())
                .and_then(|v| v.as_object().cloned())
                .ok_or_else(|| MadrpcError::InvalidRequest("JSON.stringify not found".into()))?;

            let json_str = stringify_fn.call(&json_obj, &[value.clone()], ctx)
                .map_err(|e| MadrpcError::JavaScriptExecution(format!("JSON.stringify failed: {}", e)))?;

            let json_string = json_str.as_string()
                .ok_or_else(|| MadrpcError::InvalidRequest("JSON.stringify didn't return string".into()))?
                .to_std_string()
                .map_err(|e| MadrpcError::InvalidRequest(format!("String conversion error: {:?}", e)))?;

            let parsed: JsonValue = serde_json::from_str(&json_string)
                .map_err(|e| MadrpcError::InvalidRequest(format!("JSON parse error: {}", e)))?;
            Ok(parsed)
        }
        JsValue::Symbol(_) => Ok(JsonValue::Null),
        _ => Ok(JsonValue::Null),
    }
}
