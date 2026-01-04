use quick_js::{Context};

fn main() {
    let ctx = Context::new().unwrap();

    // First, set up the madrpc object
    ctx.eval(r#"
        globalThis.madrpc = {
            _registeredFunctions: {},
            register: function(name, fn) {
                this._registeredFunctions[name] = fn;
            }
        };
    "#).unwrap();

    // Register a test function
    ctx.eval(r#"
        madrpc.register('echo', function(args) {
            return args;
        });
    "#).unwrap();

    // Test the set_global approach
    let method = "echo";
    let method_js = serde_json::json!(method).to_string();
    println!("method_js: {}", method_js);

    let args_str = r#"{"msg":"hello"}"#;
    ctx.set_global("__madrpc_args", args_str).unwrap();

    // Test the full expression
    let expr = format!(
        "JSON.stringify(madrpc._registeredFunctions[{}](JSON.parse(__madrpc_args)))",
        method_js
    );
    println!("Expression: {}", expr);

    let result = ctx.eval(&expr);
    println!("Result: {:?}", result);
}
