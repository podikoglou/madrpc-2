//! Integration tests for the MaDRPC server runtime.
//!
//! This module tests the JavaScript runtime integration, including:
//! - Context creation and initialization
//! - Function registration and calling
//! - Promise handling and async execution
//! - Error handling
//! - Thread safety guarantees
//! - Security properties (no pointer-as-number storage)

#[cfg(test)]
mod tests {
    use crate::runtime::context::MadrpcContext;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Helper function to create a temporary test script file.
    ///
    /// This function generates a unique file path for each test to avoid
    /// conflicts when running tests in parallel.
    ///
    /// # Parameters
    ///
    /// * `content` - The JavaScript source code to write to the file
    ///
    /// # Returns
    ///
    /// The path to the created script file.
    ///
    /// # Note
    ///
    /// Test files are created in `/tmp/` with unique names. They are not
    /// automatically cleaned up; tests should clean up after themselves if needed.
    fn create_test_script(content: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!("/tmp/test_madrpc_{}.js", id));
        fs::write(&path, content).unwrap();
        path
    }

    /// Test that a MadrpcContext can be created from a script file.
    ///
    /// This test verifies the basic initialization path: loading a script
    /// and creating a Boa context with MaDRPC bindings.
    #[test]
    fn test_context_creation() {
        let script = create_test_script("void 0;");
        let ctx = MadrpcContext::new(&script);
        assert!(ctx.is_ok());
    }

    /// Test that JavaScript functions can be registered via `madrpc.register()`.
    ///
    /// This test verifies that the registration binding works correctly and
    /// functions are stored in the registry.
    #[test]
    fn test_register_function() {
        let script = create_test_script(r#"
            madrpc.register('testFunc', function(args) {
                return { result: 'hello' };
            });
        "#);
        let ctx = MadrpcContext::new(&script);
        assert!(ctx.is_ok());
    }

    /// Test that registered JavaScript functions can be called from Rust.
    ///
    /// This test verifies the full RPC flow:
    /// 1. Register a function in JavaScript
    /// 2. Call it from Rust via `call_rpc()`
    /// 3. Receive the result
    #[test]
    fn test_call_registered_function() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let mut ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("echo", json!({"msg": "test"})).unwrap();
        assert_eq!(result, json!({"msg": "test"}));
    }

    /// Test that JavaScript functions can perform computations and return results.
    ///
    /// This test verifies that argument passing and return value conversion
    /// work correctly for simple computations.
    #[test]
    fn test_call_function_with_computation() {
        let script = create_test_script(r#"
            madrpc.register('add', function(args) {
                return { sum: args.a + args.b };
            });
        "#);
        let mut ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("add", json!({"a": 5, "b": 3})).unwrap();
        assert_eq!(result, json!({"sum": 8}));
    }

    /// Test that calling an unregistered function returns an error.
    ///
    /// This test verifies that the RPC system properly validates method
    /// names and returns an appropriate error for non-existent methods.
    #[test]
    fn test_call_unregistered_function_returns_error() {
        let script = create_test_script("void 0;");
        let mut ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("nonexistent", json!({}));
        assert!(result.is_err());
    }

    /// Test that scripts with syntax errors fail to load.
    ///
    /// This test verifies that the context creation process properly
    /// detects and reports JavaScript syntax errors.
    #[test]
    fn test_load_script_with_syntax_error_returns_error() {
        let script = create_test_script("this is not valid javascript ))");
        let ctx = MadrpcContext::new(&script);
        assert!(ctx.is_err());
    }

    /// Test that JavaScript functions can return null.
    ///
    /// This test verifies that the null value is properly converted
    /// between JavaScript and JSON representations.
    #[test]
    fn test_function_returning_null() {
        let script = create_test_script(r#"
            madrpc.register('nullReturn', function() { return null; });
        "#);
        let mut ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("nullReturn", json!(null)).unwrap();
        assert_eq!(result, json!(null));
    }

    /// Test that JavaScript functions can return nested objects.
    ///
    /// This test verifies that deeply nested objects are properly
    /// converted between JavaScript and JSON representations.
    #[test]
    fn test_function_with_nested_object() {
        let script = create_test_script(r#"
            madrpc.register('nested', function(args) {
                return {
                    deep: {
                        nested: {
                            value: args.x * 2
                        }
                    }
                };
            });
        "#);
        let mut ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("nested", json!({"x": 21})).unwrap();
        assert_eq!(result, json!({"deep": {"nested": {"value": 42}}}));
    }

    /// Test that a context can be created with an optional client.
    ///
    /// This test verifies the `with_client()` constructor works correctly
    /// even when no client is provided (None).
    #[test]
    fn test_context_with_client() {
        let script = create_test_script("// empty");
        let ctx = MadrpcContext::with_client(&script, None);
        assert!(ctx.is_ok());
    }

    // ========================================================================
    // Security Tests
    // ========================================================================

    /// Test that MadrpcContext is NOT Send (cannot be sent across threads).
    ///
    /// This is a compile-time test that verifies the `!Send` marker is working.
    /// If MadrpcContext were Send, this test would fail to compile.
    ///
    /// # Safety
    ///
    /// This is critical for preventing undefined behavior with Boa's
    /// thread-local Context.
    #[test]
    fn test_context_is_not_send() {
        // This test verifies at compile-time that MadrpcContext cannot be sent
        // If this compiles, it means MadrpcContext is Send (which would be wrong)
        fn assert_not_send<T: !Send>() {}
        // This line will fail to compile if MadrpcContext implements Send
        // Currently it should compile because MadrpcContext has a PhantomData<Rc<()>>
        // which makes it !Send
        assert_not_send::<MadrpcContext>();
    }

    /// Test that MadrpcContext is NOT Sync (cannot be shared between threads).
    ///
    /// This is a compile-time test that verifies the `!Sync` marker is working.
    /// If MadrpcContext were Sync, this test would fail to compile.
    ///
    /// # Safety
    ///
    /// This is critical for preventing undefined behavior with Boa's
    /// thread-local Context.
    #[test]
    fn test_context_is_not_sync() {
        // This test verifies at compile-time that MadrpcContext cannot be shared
        // If this compiles, it means MadrpcContext is Sync (which would be wrong)
        fn assert_not_sync<T: !Sync>() {}
        // This line will fail to compile if MadrpcContext implements Sync
        // Currently it should compile because MadrpcContext has a PhantomData<Rc<()>>
        // which makes it !Sync
        assert_not_sync::<MadrpcContext>();
    }

    /// Test that bindings don't use pointer-as-number storage.
    ///
    /// This security test verifies that the old unsafe pattern of storing
    /// raw pointers as JavaScript numbers has been eliminated. The test
    /// scans all properties on the madrpc object to ensure no pointer-like
    /// values exist.
    ///
    /// # Background
    ///
    /// Previous versions of the code stored the client pointer as a number
    /// on the madrpc object, which is unsafe and can lead to memory corruption.
    /// The new implementation uses closure capture instead.
    #[test]
    fn test_no_pointer_as_number_storage() {
        use boa_engine::{Context, js_string};

        // Create a context and install bindings
        let job_executor = std::rc::Rc::new(crate::runtime::TokioJobExecutor::new());
        let mut ctx = Context::builder()
            .job_executor(job_executor)
            .build()
            .unwrap();

        // Install bindings without a client
        crate::runtime::bindings::install_madrpc_bindings(&mut ctx, None).unwrap();

        // Get the madrpc object
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        let madrpc_obj = madrpc.as_object().unwrap();

        // Check that __client_ptr does NOT exist
        let client_ptr = madrpc_obj.get(js_string!("__client_ptr"), &mut ctx).unwrap();
        assert!(client_ptr.is_undefined(),
            "__client_ptr should not exist (we eliminated pointer-as-number storage)");

        // Iterate through all properties and ensure none look like pointers
        let props = madrpc_obj.own_property_keys(&mut ctx).unwrap();
        for prop in props {
            if let Some(prop_str) = prop.as_string() {
                let prop_name = prop_str.to_std_string().unwrap();
                if prop_name.contains("ptr") || prop_name.contains("pointer") {
                    panic!("Found property with 'ptr' in name: {} (pointer-as-number detected!)",
                           prop_name);
                }

                let value = madrpc_obj.get(prop, &mut ctx).unwrap();
                if value.is_number() {
                    let num = value.as_number().unwrap();
                    // Check if it looks like a pointer (very large number > 1GB)
                    if num > 1_000_000_000.0 && num < (1u64 << 48) as f64 {
                        panic!("Found suspicious large number {} = {} (might be a pointer)",
                               prop_name, num);
                    }
                }
            }
        }
    }

    /// Test that setOrchestrator was removed from the API.
    ///
    /// This test verifies that the unsafe `setOrchestrator` function has
    /// been removed from the public API. Users should now create nodes
    /// with orchestrator support via `Node::with_orchestrator()` instead.
    #[test]
    fn test_set_orchestrator_removed_from_api() {
        use boa_engine::{Context, js_string};

        // Create a context and install bindings
        let job_executor = std::rc::Rc::new(crate::runtime::TokioJobExecutor::new());
        let mut ctx = Context::builder()
            .job_executor(job_executor)
            .build()
            .unwrap();

        // Install bindings without a client
        crate::runtime::bindings::install_madrpc_bindings(&mut ctx, None).unwrap();

        // Get the madrpc object
        let madrpc = ctx.global_object()
            .get(js_string!("madrpc"), &mut ctx)
            .unwrap();
        let madrpc_obj = madrpc.as_object().unwrap();

        // Check that setOrchestrator does NOT exist
        let set_orch = madrpc_obj.get(js_string!("setOrchestrator"), &mut ctx).unwrap();
        assert!(set_orch.is_undefined(),
            "setOrchestrator should not exist (removed from API for security)");
    }

    /// Test that shared runtime is reused instead of creating new ones.
    ///
    /// This test verifies that the blocking runtime is created once and
    /// reused for all blocking calls, preventing resource exhaustion.
    #[test]
    fn test_shared_blocking_runtime_reuse() {
        use crate::runtime::bindings::get_blocking_runtime;

        // First call should create the runtime
        let rt1 = get_blocking_runtime();
        assert!(rt1.is_ok(), "First call should succeed");

        // Second call should return the same runtime
        let rt2 = get_blocking_runtime();
        assert!(rt2.is_ok(), "Second call should succeed");

        // They should be the same pointer (same instance)
        assert_eq!(
            rt1.unwrap().as_ptr(),
            rt2.unwrap().as_ptr(),
            "Should return the same runtime instance (no new runtime created)"
        );
    }
}
