#[cfg(test)]
mod tests {
    use crate::runtime::context::MadrpcContext;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn create_test_script(content: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!("/tmp/test_madrpc_{}.js", id));
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_context_creation() {
        let script = create_test_script("void 0;");
        let ctx = MadrpcContext::new(&script);
        assert!(ctx.is_ok());
    }

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

    #[test]
    fn test_call_registered_function() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("echo", json!({"msg": "test"})).unwrap();
        assert_eq!(result, json!({"msg": "test"}));
    }

    #[test]
    fn test_call_function_with_computation() {
        let script = create_test_script(r#"
            madrpc.register('add', function(args) {
                return { sum: args.a + args.b };
            });
        "#);
        let ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("add", json!({"a": 5, "b": 3})).unwrap();
        assert_eq!(result, json!({"sum": 8}));
    }

    #[test]
    fn test_call_unregistered_function_returns_error() {
        let script = create_test_script("void 0;");
        let ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("nonexistent", json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_script_with_syntax_error_returns_error() {
        let script = create_test_script("this is not valid javascript ))");
        let ctx = MadrpcContext::new(&script);
        assert!(ctx.is_err());
    }

    #[test]
    fn test_function_returning_null() {
        let script = create_test_script(r#"
            madrpc.register('nullReturn', function() { return null; });
        "#);
        let ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("nullReturn", json!(null)).unwrap();
        assert_eq!(result, json!(null));
    }

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
        let ctx = MadrpcContext::new(&script).unwrap();

        let result = ctx.call_rpc("nested", json!({"x": 21})).unwrap();
        assert_eq!(result, json!({"deep": {"nested": {"value": 42}}}));
    }

    #[test]
    fn test_context_with_client() {
        let script = create_test_script("// empty");
        let ctx = MadrpcContext::with_client(&script, None);
        assert!(ctx.is_ok());
    }

    // ========================================================================
    // Security Tests
    // ========================================================================

    /// Test that MadrpcContext is NOT Send (cannot be sent across threads)
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

    /// Test that MadrpcContext is NOT Sync (cannot be shared between threads)
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

    /// Test that bindings don't use pointer-as-number storage
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

    /// Test that setOrchestrator was removed from the API
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

    /// Test that shared runtime is reused instead of creating new ones
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
