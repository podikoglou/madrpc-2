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
}
