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

    /// number of QuickJS contexts in the pool (default: num_cpus)
    #[argh(option, long = "pool-size")]
    pool_size: Option<usize>,
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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli: Cli = argh::from_env();

    match cli.command {
        Commands::Node(args) => {
            tracing::info!("Starting MaDRPC node with script: {}", args.script);
            tracing::info!("Binding to: {}", args.bind);

            // Determine max threads (use CLI flag or default to num_cpus * 2)
            let max_threads = args.pool_size.unwrap_or_else(|| num_cpus::get() * 2);
            tracing::info!("Max worker threads: {}", max_threads);

            // Create the node with thread limit
            let node = madrpc_server::Node::with_thread_limit(
                std::path::PathBuf::from(&args.script),
                max_threads
            )?;
            tracing::info!("Node created successfully from script");

            // Create TCP threaded server
            let server = madrpc_common::transport::tcp_server::TcpServerThreaded::new(&args.bind, max_threads)?;
            let actual_addr = server.local_addr()?;
            tracing::info!("TCP server listening on {}", actual_addr);

            // Run server with node handler (synchronous)
            server.run_with_handler(move |request| {
                node.handle_request(&request)
            })?;

            Ok(())
        }
        Commands::Orchestrator(args) => {
            tracing::info!("Starting MaDRPC orchestrator");
            tracing::info!("Binding to: {}", args.bind);
            tracing::info!("Nodes: {:?}", args.nodes);

            if args.nodes.is_empty() {
                tracing::warn!("No nodes specified! Use --node <addr> to add nodes.");
            }

            let orch = madrpc_orchestrator::Orchestrator::new(args.nodes).await?;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_node() {
        let args: Cli = Cli::from_args(&["madrpc"], &["node", "-s", "test.js", "-b", "0.0.0.0:9001"]).unwrap();
        match args.command {
            Commands::Node(NodeArgs { script, bind, pool_size }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
                assert!(pool_size.is_none());
            }
            _ => panic!("Expected Node command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_single_node() {
        let args: Cli = Cli::from_args(&["madrpc"], &["orchestrator", "-n", "localhost:9001"]).unwrap();
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
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "orchestrator",
            "--node", "localhost:9001",
            "--node", "localhost:9002",
            "--node", "localhost:9003",
        ]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind: _, nodes }) => {
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
}
