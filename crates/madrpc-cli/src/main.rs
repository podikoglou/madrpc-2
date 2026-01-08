//! # MaDRPC CLI Entry Point
//!
//! Main binary for the MaDRPC distributed RPC system. Provides command-line
//! interface for starting nodes, orchestrators, monitoring, and making RPC calls.
//!
//! ## Usage
//!
//! ```bash
//! # Start a node
//! madrpc node -s script.js -b 0.0.0.0:9001
//!
//! # Start an orchestrator
//! madrpc orchestrator -b 0.0.0.0:8080 -n http://127.0.0.1:9001 -n http://127.0.0.1:9002
//!
//! # Monitor with real-time metrics
//! madrpc top http://127.0.0.1:8080
//!
//! # Make an RPC call (outputs raw JSON)
//! madrpc call http://127.0.0.1:8080 method_name '{"arg": "value"}'
//! ```

use argh::FromArgs;
use anyhow::Result;
use std::net::SocketAddr;

/// Main CLI structure parsed from command-line arguments.
///
/// Uses `argh` for declarative argument parsing. The top-level command
/// dispatches to one of the four subcommands: node, orchestrator, top, or call.
#[derive(FromArgs)]
/// MaDRPC - Massively Distributed RPC system
struct Cli {
    #[argh(subcommand)]
    command: Commands,
}

/// Available CLI subcommands.
///
/// Each variant represents a distinct operational mode:
///
/// - **Node**: Start a JavaScript execution server
/// - **Orchestrator**: Start a load-balancing request forwarder
/// - **Top**: Monitor a server with real-time metrics TUI
/// - **Call**: Make a single RPC call (unix-friendly JSON output)
#[derive(FromArgs)]
#[argh(subcommand)]
enum Commands {
    Node(NodeArgs),
    Orchestrator(OrchestratorArgs),
    Top(TopArgs),
    Call(CallArgs),
}

/// Arguments for starting a MaDRPC node.
///
/// Nodes are JavaScript execution servers that load a script file and expose
/// registered functions via JSON-RPC over HTTP. Each node maintains its own Boa JavaScript
/// context and processes requests on the configured bind address.
///
/// # Example
///
/// ```bash
/// madrpc node -s examples/monte-carlo-pi/scripts/pi.js -b 0.0.0.0:9001
/// ```
#[derive(FromArgs)]
#[argh(subcommand, name = "node")]
/// start a MaDRPC node
struct NodeArgs {
    /// path to the JavaScript file to load and execute
    ///
    /// The script should use `madrpc.register(name, function)` to expose
    /// RPC methods. The file is read once at startup and cached.
    #[argh(option, short = 's')]
    script: String,

    /// address to bind the node's HTTP server to
    ///
    /// Defaults to "0.0.0.0:0" which assigns a random available port.
    /// The actual bound address is logged at startup.
    #[argh(option, short = 'b', default = "\"0.0.0.0:0\".into()")]
    bind: String,

    /// optional orchestrator address to register with
    ///
    /// If provided, the node will register itself with the orchestrator
    /// for automatic discovery. Otherwise, the node operates standalone.
    #[argh(option, long = "orchestrator")]
    orchestrator: Option<String>,
}

/// Arguments for starting a MaDRPC orchestrator.
///
/// Orchestrators are load balancers that forward JSON-RPC over HTTP requests to registered
/// nodes using round-robin selection. They provide health checking and
/// circuit breaker functionality for high availability.
///
/// # Health Checking
///
/// The orchestrator periodically pings nodes to verify liveness. After the
/// configured number of consecutive failures, a node is removed from the
/// rotation until it recovers.
///
/// # Example
///
/// ```bash
/// madrpc orchestrator -b 0.0.0.0:8080 \
///   -n http://127.0.0.1:9001 \
///   -n http://127.0.0.1:9002 \
///   --health-check-interval 10 \
///   --health-check-timeout 3000
/// ```
#[derive(FromArgs)]
#[argh(subcommand, name = "orchestrator")]
/// start a MaDRPC orchestrator
struct OrchestratorArgs {
    /// address to bind the orchestrator's HTTP server to
    ///
    /// Clients connect to this address to make RPC calls. Defaults to
    /// "0.0.0.0:8080" for accessibility from other machines.
    #[argh(option, short = 'b', default = "\"0.0.0.0:8080\".into()")]
    bind: String,

    /// addresses of nodes to forward requests to
    ///
    /// Can be specified multiple times to add multiple nodes. Requests are
    /// distributed using round-robin load balancing. At least one node is
    /// recommended for useful operation.
    #[argh(option, short = 'n', long = "node")]
    nodes: Vec<String>,

    /// interval between health checks in seconds
    ///
    /// The orchestrator pings each node at this interval to verify liveness.
    /// Defaults to 5 seconds. Lower values detect failures faster but increase
    /// network load.
    #[argh(option, long = "health-check-interval", default = "5")]
    health_check_interval_secs: u64,

    /// timeout for each health check in milliseconds
    ///
    /// If a node doesn't respond within this time, the health check is
    /// considered a failure. Defaults to 2000ms (2 seconds).
    #[argh(option, long = "health-check-timeout", default = "2000")]
    health_check_timeout_ms: u64,

    /// consecutive health check failures before disabling a node
    ///
    /// After this many failures in a row, the node is removed from the
    /// rotation until it recovers (circuit breaker pattern). Defaults to 3.
    #[argh(option, long = "health-check-failure-threshold", default = "3")]
    health_check_failure_threshold: u32,

    /// disable health checking entirely
    ///
    /// When set, the orchestrator will not ping nodes and will continue
    /// forwarding to all configured nodes regardless of their health.
    /// Useful for testing or environments with unreliable networks.
    #[argh(switch, long = "disable-health-check")]
    disable_health_check: bool,
}

/// Arguments for the real-time metrics monitoring TUI.
///
/// The `top` command displays a live dashboard showing request metrics,
/// latency percentiles, active connections, and uptime. For orchestrators,
/// it also shows per-node request distribution.
///
/// # Controls
///
/// Press `q` or `Q` to quit the TUI.
///
/// # Example
///
/// ```bash
/// # Monitor orchestrator (shows node distribution)
/// madrpc top 127.0.0.1:8080
///
/// # Monitor node with slower refresh
/// madrpc top --interval 1000 127.0.0.1:9001
/// ```
#[derive(FromArgs)]
#[argh(subcommand, name = "top")]
/// monitor a MaDRPC server with real-time metrics
struct TopArgs {
    /// address of the server to monitor
    ///
    /// Can be an orchestrator or a standalone node. The TUI automatically
    /// detects the server type and adjusts the display accordingly.
    #[argh(positional)]
    server_address: String,

    /// refresh interval in milliseconds
    ///
    /// Controls how frequently the TUI fetches updated metrics from the
    /// server. Lower values provide more responsive updates but increase
    /// network and CPU usage. Defaults to 250ms.
    #[argh(option, short = 'i', long = "interval", default = "250")]
    interval_ms: u64,
}

/// Arguments for making a single RPC call.
///
/// The `call` command makes one RPC call and outputs the result as raw JSON
/// to stdout. This makes it suitable for scripting and integration with
/// other tools (e.g., `jq`, `awk`, etc.).
///
/// # Output Format
///
/// Outputs raw JSON (no pretty-printing) to stdout. Errors are reported
/// to stderr with non-zero exit code.
///
/// # Examples
///
/// ```bash
/// # Call a method with no arguments
/// madrpc call 127.0.0.1:8080 get_status
///
/// # Call with arguments
/// madrpc call 127.0.0.1:8080 monte_carlo '{"samples": 1000000}'
///
/// # Pipe output to jq for processing
/// madrpc call 127.0.0.1:8080 get_stats | jq '.pi_estimate'
/// ```
#[derive(FromArgs)]
#[argh(subcommand, name = "call")]
/// call an RPC method on a server
struct CallArgs {
    /// address of the server to call
    ///
    /// Can be an orchestrator (which load balances) or a specific node.
    #[argh(positional)]
    server_address: String,

    /// name of the RPC method to call
    ///
    /// Must match a name registered via `madrpc.register()` in the node's
    /// JavaScript script.
    #[argh(positional)]
    method: String,

    /// JSON string containing arguments for the method
    ///
    /// Must be valid JSON. Use empty object `{}` for methods with no arguments.
    /// Defaults to `{}`.
    #[argh(option, short = 'a', long = "args", default = "\"{}\".into()")]
    args: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli: Cli = argh::from_env();

    // Initialize tracing only for non-call commands (to keep output clean for unix tool usage)
    if !matches!(cli.command, Commands::Call(_)) {
        // Set default log level to INFO, but allow RUST_LOG env var to override
        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }

    match cli.command {
        Commands::Node(args) => {
            tracing::info!("Starting MaDRPC node with script: {}", args.script);
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

            let server = madrpc_server::HttpServer::new(std::sync::Arc::new(node));
            let addr: SocketAddr = args.bind.parse()
                .map_err(|e| anyhow::anyhow!("Invalid bind address {}: {}", args.bind, e))?;
            server.run(addr).await?;

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

            let server = madrpc_orchestrator::HttpServer::new(std::sync::Arc::new(orch));
            let addr: SocketAddr = args.bind.parse()
                .map_err(|e| anyhow::anyhow!("Invalid bind address {}: {}", args.bind, e))?;
            server.run(addr).await?;

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

/// Executes the `call` subcommand.
///
/// This function:
/// 1. Parses the JSON arguments string
/// 2. Creates a client connection to the server
/// 3. Makes the RPC call
/// 4. Outputs the raw JSON result to stdout
///
/// No tracing/logging is initialized for this command to keep output clean
/// for unix tool usage (piping to jq, etc.).
///
/// # Errors
///
/// Returns an error if:
/// - The args string is not valid JSON
/// - The connection to the server fails
/// - The RPC call itself fails
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

/// CLI argument parsing tests.
///
/// Tests verify that `argh` correctly parses all subcommands and their
/// arguments. Each test simulates command-line invocation and validates
/// the resulting structure.
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
