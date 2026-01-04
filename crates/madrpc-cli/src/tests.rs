#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_node() {
        let args: Cli = argh::from_args(&["madrpc", "node", "-s", "test.js", "-b", "0.0.0.0:9001"]).unwrap();
        match args.command {
            Commands::Node(NodeArgs { script, bind }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
            }
            _ => panic!("Expected Node command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_single_node() {
        let args: Cli = argh::from_args(&["madrpc", "orchestrator", "-n", "localhost:9001"]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind, nodes }) => {
                assert_eq!(bind, "0.0.0.0:8080"); // default
                assert_eq!(nodes, vec!["localhost:9001".to_string()]);
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_multiple_nodes() {
        let args: Cli = argh::from_args(&[
            "madrpc", "orchestrator",
            "--node", "localhost:9001",
            "--node", "localhost:9002",
            "--node", "localhost:9003",
        ]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind, nodes }) => {
                assert_eq!(nodes, vec![
                    "localhost:9001".to_string(),
                    "localhost:9002".to_string(),
                    "localhost:9003".to_string(),
                ]);
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_no_nodes() {
        let args: Cli = argh::from_args(&["madrpc", "orchestrator"]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { nodes, .. }) => {
                assert!(nodes.is_empty());
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_custom_bind() {
        let args: Cli = argh::from_args(&[
            "madrpc", "orchestrator",
            "--bind", "0.0.0.0:9000",
        ]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind, .. }) => {
                assert_eq!(bind, "0.0.0.0:9000");
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }
}
