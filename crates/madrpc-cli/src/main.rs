use argh::FromArgs;
use anyhow::Result;

#[derive(FromArgs)]
/// MaDRPC - Massively Distributed RPC system
struct Cli {
    #[argh(subcommand)]
    command: Commands,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Commands {
    Node(NodeArgs),
    Orchestrator(OrchestratorArgs),
    Top(TopArgs),
    Call(CallArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "node")]
/// start a MaDRPC node
struct NodeArgs {
    /// javascript file to load
    #[argh(option, short = 's')]
    script: String,

    /// address to bind to
    #[argh(option, short = 'b', default = "\"0.0.0.0:0\".into()")]
    bind: String,

    /// orchestrator address to register with
    #[argh(option, long = "orchestrator")]
    orchestrator: Option<String>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "orchestrator")]
/// start a MaDRPC orchestrator
struct OrchestratorArgs {
    /// address to bind to
    #[argh(option, short = 'b', default = "\"0.0.0.0:8080\".into()")]
    bind: String,

    /// node addresses (can be specified multiple times)
    #[argh(option, short = 'n', long = "node")]
    nodes: Vec<String>,

    /// health check interval in seconds (default: 5)
    #[argh(option, long = "health-check-interval", default = "5")]
    health_check_interval_secs: u64,

    /// health check timeout in milliseconds (default: 2000)
    #[argh(option, long = "health-check-timeout", default = "2000")]
    health_check_timeout_ms: u64,

    /// consecutive health check failures before disabling node (default: 3)
    #[argh(option, long = "health-check-failure-threshold", default = "3")]
    health_check_failure_threshold: u32,

    /// disable health checking entirely
    #[argh(switch, long = "disable-health-check")]
    disable_health_check: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "top")]
/// monitor a MaDRPC server with real-time metrics
struct TopArgs {
    /// address of the server to monitor
    #[argh(positional)]
    server_address: String,

    /// refresh interval in milliseconds
    #[argh(option, short = 'i', long = "interval", default = "250")]
    interval_ms: u64,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "call")]
/// call an RPC method on a server
struct CallArgs {
    /// address of the server to call
    #[argh(positional)]
    server_address: String,

    /// name of the method to call
    #[argh(positional)]
    method: String,

    /// JSON string containing arguments for the method
    #[argh(option, short = 'a', long = "args", default = "\"{}\".into()")]
    args: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli: Cli = argh::from_env();

    // Initialize tracing only for non-call commands (to keep output clean for unix tool usage)
    if !matches!(cli.command, Commands::Call(_)) {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    match cli.command {
        Commands::Node(args) => {
            tracing::info!("Starting MaDRPC node (single-threaded) with script: {}", args.script);
            tracing::info!("Binding to: {}", args.bind);

            let node = if let Some(orch_addr) = &args.orchestrator {
                tracing::info!("Configured with orchestrator: {}", orch_addr);
                madrpc_server::Node::with_orchestrator(
                    std::path::PathBuf::from(&args.script),
                    orch_addr.clone(),
                ).await?
            } else {
                madrpc_server::Node::new(std::path::PathBuf::from(&args.script))?
            };
            tracing::info!("Node created successfully from script");

            let server = madrpc_common::transport::tcp_server::TcpServer::new(&args.bind).await?;
            let actual_addr = server.local_addr()?;
            tracing::info!("TCP server listening on {} (single-threaded)", actual_addr);

            let node = std::sync::Arc::new(node);
            server.run_with_handler(move |request| {
                let node = node.clone();
                async move {
                    node.handle_request(&request)
                }
            }).await?;

            Ok(())
        }
        Commands::Orchestrator(args) => {
            tracing::info!("Starting MaDRPC orchestrator");
            tracing::info!("Binding to: {}", args.bind);
            tracing::info!("Nodes: {:?}", args.nodes);

            if args.nodes.is_empty() {
                tracing::warn!("No nodes specified! Use --node <addr> to add nodes.");
            }

            let orch = if args.disable_health_check {
                tracing::info!("Health checking disabled");
                // Use a very long interval to effectively disable health checks
                let config = madrpc_orchestrator::HealthCheckConfig {
                    interval: std::time::Duration::from_secs(365 * 24 * 60 * 60), // 1 year
                    timeout: std::time::Duration::from_millis(args.health_check_timeout_ms),
                    failure_threshold: args.health_check_failure_threshold,
                };
                madrpc_orchestrator::Orchestrator::with_config(args.nodes, config).await?
            } else {
                tracing::info!("Health check interval: {}s", args.health_check_interval_secs);
                tracing::info!("Health check timeout: {}ms", args.health_check_timeout_ms);
                tracing::info!("Health check failure threshold: {}", args.health_check_failure_threshold);

                let config = madrpc_orchestrator::HealthCheckConfig {
                    interval: std::time::Duration::from_secs(args.health_check_interval_secs),
                    timeout: std::time::Duration::from_millis(args.health_check_timeout_ms),
                    failure_threshold: args.health_check_failure_threshold,
                };
                madrpc_orchestrator::Orchestrator::with_config(args.nodes, config).await?
            };

            tracing::info!("Orchestrator created with {} nodes", orch.node_count().await);

            // Create TCP async server
            let server = madrpc_common::transport::tcp_server::TcpServer::new(&args.bind).await?;
            let actual_addr = server.local_addr()?;
            tracing::info!("TCP server listening on {}", actual_addr);

            // Run server with orchestrator handler (async)
            let orch_ref = std::sync::Arc::new(orch);
            server.run_with_handler(move |request| {
                let orch = orch_ref.clone();
                async move {
                    orch.forward_request(&request).await
                }
            }).await?;

            Ok(())
        }
        Commands::Top(args) => {
            madrpc_cli::top::run_top(args.server_address, args.interval_ms).await
        }
        Commands::Call(args) => {
            run_call(args).await
        }
    }
}

/// Run the call command - outputs raw JSON to stdout
async fn run_call(args: CallArgs) -> Result<()> {
    // Parse args JSON string
    let args_value: serde_json::Value = serde_json::from_str(&args.args)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in args: {}", e))?;

    // Create client and call method
    let client = madrpc_client::MadrpcClient::new(&args.server_address).await?;
    let result = client.call(&args.method, args_value).await?;

    // Output raw JSON to stdout
    println!("{}", serde_json::to_string(&result)?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_node() {
        let args: Cli = Cli::from_args(&["madrpc"], &["node", "-s", "test.js", "-b", "0.0.0.0:9001"]).unwrap();
        match args.command {
            Commands::Node(NodeArgs { script, bind, orchestrator }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
                assert!(orchestrator.is_none());
            }
            _ => panic!("Expected Node command"),
        }
    }

    #[test]
    fn test_cli_parse_node_with_orchestrator() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "node",
            "-s", "test.js",
            "-b", "0.0.0.0:9001",
            "--orchestrator", "127.0.0.1:8080",
        ]).unwrap();
        match args.command {
            Commands::Node(NodeArgs { script, bind, orchestrator }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
                assert_eq!(orchestrator, Some("127.0.0.1:8080".to_string()));
            }
            _ => panic!("Expected Node command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_single_node() {
        let args: Cli = Cli::from_args(&["madrpc"], &["orchestrator", "-n", "localhost:9001"]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind, nodes, .. }) => {
                assert_eq!(bind, "0.0.0.0:8080"); // default
                assert_eq!(nodes, vec!["localhost:9001".to_string()]);
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_multiple_nodes() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "orchestrator",
            "--node", "localhost:9001",
            "--node", "localhost:9002",
            "--node", "localhost:9003",
        ]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { nodes, .. }) => {
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
        let args: Cli = Cli::from_args(&["madrpc"], &["orchestrator"]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { nodes, .. }) => {
                assert!(nodes.is_empty());
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_custom_bind() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "orchestrator",
            "--bind", "0.0.0.0:9000",
        ]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind, .. }) => {
                assert_eq!(bind, "0.0.0.0:9000");
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_top() {
        let args: Cli = Cli::from_args(&["madrpc"], &["top", "127.0.0.1:8080"]).unwrap();
        match args.command {
            Commands::Top(TopArgs { server_address, interval_ms }) => {
                assert_eq!(server_address, "127.0.0.1:8080");
                assert_eq!(interval_ms, 250); // default
            }
            _ => panic!("Expected Top command"),
        }
    }

    #[test]
    fn test_cli_parse_top_with_interval() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "top",
            "--interval", "500",
            "127.0.0.1:8080",
        ]).unwrap();
        match args.command {
            Commands::Top(TopArgs { server_address, interval_ms }) => {
                assert_eq!(server_address, "127.0.0.1:8080");
                assert_eq!(interval_ms, 500);
            }
            _ => panic!("Expected Top command"),
        }
    }

    #[test]
    fn test_cli_parse_call() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "call",
            "127.0.0.1:8080",
            "test_method",
        ]).unwrap();
        match args.command {
            Commands::Call(CallArgs { server_address, method, args }) => {
                assert_eq!(server_address, "127.0.0.1:8080");
                assert_eq!(method, "test_method");
                assert_eq!(args, "{}"); // default
            }
            _ => panic!("Expected Call command"),
        }
    }

    #[test]
    fn test_cli_parse_call_with_args() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "call",
            "127.0.0.1:8080",
            "test_method",
            "--args", "{\"key\":\"value\"}",
        ]).unwrap();
        match args.command {
            Commands::Call(CallArgs { server_address, method, args }) => {
                assert_eq!(server_address, "127.0.0.1:8080");
                assert_eq!(method, "test_method");
                assert_eq!(args, "{\"key\":\"value\"}");
            }
            _ => panic!("Expected Call command"),
        }
    }

    #[test]
    fn test_cli_parse_call_with_short_args() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "call",
            "127.0.0.1:8080",
            "test_method",
            "-a", "{\"samples\":1000}",
        ]).unwrap();
        match args.command {
            Commands::Call(CallArgs { server_address, method, args }) => {
                assert_eq!(server_address, "127.0.0.1:8080");
                assert_eq!(method, "test_method");
                assert_eq!(args, "{\"samples\":1000}");
            }
            _ => panic!("Expected Call command"),
        }
    }
}
