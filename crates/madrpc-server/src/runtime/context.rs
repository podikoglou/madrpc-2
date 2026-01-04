use quick_js::{Context, JsValue};
use std::path::Path;
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};

/// QuickJS context wrapper with MaDRPC bindings
pub struct MadrpcContext {
    ctx: Mutex<Context>,
    script_path: String,
    registered_functions: Vec<String>,
    client: Option<Arc<madrpc_client::MadrpcClient>>,
}

// Safety: QuickJS Context is not thread-safe, so we wrap it in a Mutex
// The Mutex ensures that only one thread can access the context at a time
unsafe impl Send for MadrpcContext {}
unsafe impl Sync for MadrpcContext {}

impl MadrpcContext {
    /// Create a new QuickJS context with MaDRPC bindings
    pub fn new(script_path: impl AsRef<Path>) -> Result<Self> {
        Self::with_client(script_path, None)
    }

    /// Create a new QuickJS context with optional client for distributed calls
    pub fn with_client(
        script_path: impl AsRef<Path>,
        client: Option<madrpc_client::MadrpcClient>,
    ) -> Result<Self> {
        let ctx = Context::new()
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        let script_path = script_path.as_ref().to_string_lossy().to_string();
        let client = client.map(Arc::new);

        let mut context = Self {
            ctx: Mutex::new(ctx),
            script_path,
            registered_functions: Vec::new(),
            client,
        };

        // Install madrpc global object
        context.install_madrpc_bindings()?;

        // Load and evaluate the script
        context.load_script()?;

        Ok(context)
    }

    /// Install the madrpc global object and functions
    fn install_madrpc_bindings(&mut self) -> Result<()> {
        // Create global madrpc object with register function
        let js_code = r#"
            globalThis.madrpc = {
                _registeredFunctions: {},
                register: function(name, fn) {
                    if (typeof fn !== 'function') {
                        throw new Error('Second argument must be a function');
                    }
                    this._registeredFunctions[name] = fn;
                }
            };
        "#;

        self.ctx.lock().unwrap().eval(js_code)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        Ok(())
    }

    /// Load and evaluate the JavaScript script
    fn load_script(&mut self) -> Result<()> {
        let script = std::fs::read_to_string(&self.script_path)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to load script: {}", e)))?;

        // Evaluate the script directly (no strict mode wrapper for now)
        self.ctx.lock().unwrap().eval(&script)
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        Ok(())
    }

    /// Call a registered RPC function
    pub fn call_rpc(&self, method: &str, args: JsonValue) -> Result<JsonValue> {
        let ctx = self.ctx.lock().unwrap();

        // Check if function is registered
        let method_js = json_to_js_string(&method.to_string());
        let check = ctx.eval(&format!(
            "typeof madrpc._registeredFunctions[{}]",
            method_js
        )).map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        if check != JsValue::String("function".to_string()) {
            return Err(MadrpcError::InvalidRequest(format!(
                "Method '{}' is not registered", method
            )));
        }

        // Set args as a global variable to avoid string escaping issues
        let args_str = serde_json::to_string(&args)
            .map_err(|e| MadrpcError::JsonSerialization(e))?;

        ctx.set_global("__madrpc_args", args_str.as_str())
            .map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        // Call the function using the global variable
        let result = ctx.eval(&format!(
            "JSON.stringify(madrpc._registeredFunctions[{}](JSON.parse(__madrpc_args)))",
            method_js
        )).map_err(|e| MadrpcError::JavaScriptExecution(e.to_string()))?;

        // Convert result back to JSON
        if let JsValue::String(result_str) = result {
            let json: JsonValue = serde_json::from_str(&result_str)
                .map_err(|e| MadrpcError::JsonSerialization(e))?;
            Ok(json)
        } else {
            Ok(JsonValue::Null)
        }
    }

    /// Get list of registered function names
    pub fn registered_functions(&self) -> &[String] {
        &self.registered_functions
    }

    /// Make a distributed RPC call (callable from JavaScript via a callback)
    pub async fn distributed_call(&self, method: &str, args: JsonValue) -> Result<JsonValue> {
        let client = self.client.as_ref()
            .ok_or_else(|| MadrpcError::InvalidRequest("No client configured for distributed calls".to_string()))?;

        client.call(method, args).await
    }
}

/// Convert a string to a JavaScript string literal using JSON.stringify
fn json_to_js_string(s: &str) -> String {
    // Use serde_json to escape the string as JSON, which is also valid JS
    let json_str = serde_json::json!(s);
    json_str.to_string()
}
