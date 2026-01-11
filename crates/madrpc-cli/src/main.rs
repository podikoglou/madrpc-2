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
//! # Start a node with orchestrator support
//! madrpc node -s script.js -b 0.0.0.0:9001 --orchestrator http://127.0.0.1:8080
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
//!
//! ## URL Format
//!
//! All URLs must include the `http://` or `https://` prefix:
//! - ✅ `http://127.0.0.1:8080`
//! - ✅ `https://example.com:8080`
//! - ❌ `127.0.0.1:8080`

use argh::FromArgs;
use anyhow::Result;
use std::net::SocketAddr;

/// Validates that a URL string starts with http:// or https://
///
/// # Arguments
///
/// * `url` - The URL string to validate
/// * `description` - Human-readable description of what the URL is for (e.g., "node address")
///
/// # Returns
///
/// `Ok(())` if the URL is valid, `Err` otherwise
///
/// # Errors
///
/// Returns an error if the URL doesn't start with http:// or https://
fn validate_http_url(url: &str, description: &str) -> Result<()> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Invalid {}: '{}' must start with http:// or https://",
            description,
            url
        ))
    }
}

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
    /// Must include the http:// or https:// prefix (e.g., http://127.0.0.1:8080).
    #[argh(option, long = "orchestrator")]
    orchestrator: Option<String>,

    /// optional API key for authentication
    ///
    /// If provided, all requests must include this key in the X-API-Key header.
    /// Authentication is disabled by default for backward compatibility.
    #[argh(option, long = "api-key")]
    api_key: Option<String>,

    /// optional rate limit in requests per second
    ///
    /// If provided, limits each IP to this many requests per second.
    /// Burst size is automatically set to 2x the rate.
    /// Rate limiting is disabled by default for backward compatibility.
    #[argh(option, long = "rate-limit-rps")]
    rate_limit_rps: Option<f64>,

    /// optional maximum execution time in milliseconds
    ///
    /// If provided, limits JavaScript execution to this many milliseconds.
    /// Prevents runaway code from consuming excessive CPU time.
    /// Defaults to 30000ms (30 seconds). Must be between 1 and 3600000 (1 hour).
    #[argh(option, long = "max-execution-time-ms", default = "30000")]
    max_execution_time_ms: u64,

    /// public URL for orchestrator registration
    ///
    /// Overrides auto-detection. If not set, tries MADRPC_PUBLIC_URL env var,
    /// then auto-detects from bind address. Required when using --orchestrator.
    #[argh(option, long = "public-url")]
    public_url: Option<String>,
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
    /// Must include the http:// or https:// prefix (e.g., http://127.0.0.1:8081).
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

    /// optional API key for authentication
    ///
    /// If provided, all requests must include this key in the X-API-Key header.
    /// Authentication is disabled by default for backward compatibility.
    #[argh(option, long = "api-key")]
    api_key: Option<String>,

    /// optional rate limit in requests per second
    ///
    /// If provided, limits each IP to this many requests per second.
    /// Burst size is automatically set to 2x the rate.
    /// Rate limiting is disabled by default for backward compatibility.
    #[argh(option, long = "rate-limit-rps")]
    rate_limit_rps: Option<f64>,
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
/// madrpc top http://127.0.0.1:8080
///
/// # Monitor node with slower refresh
/// madrpc top --interval 1000 http://127.0.0.1:9001
/// ```
#[derive(FromArgs)]
#[argh(subcommand, name = "top")]
/// monitor a MaDRPC server with real-time metrics
struct TopArgs {
    /// address of the server to monitor
    ///
    /// Can be an orchestrator or a standalone node. The TUI automatically
    /// detects the server type and adjusts the display accordingly.
    /// Must include the http:// or https:// prefix (e.g., http://127.0.0.1:8080).
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
/// madrpc call http://127.0.0.1:8080 get_status
///
/// # Call with arguments
/// madrpc call http://127.0.0.1:8080 monte_carlo '{"samples": 1000000}'
///
/// # Pipe output to jq for processing
/// madrpc call http://127.0.0.1:8080 get_stats | jq '.pi_estimate'
/// ```
#[derive(FromArgs)]
#[argh(subcommand, name = "call")]
/// call an RPC method on a server
struct CallArgs {
    /// address of the server to call
    ///
    /// Can be an orchestrator (which load balances) or a specific node.
    /// Must include the http:// or https:// prefix (e.g., http://127.0.0.1:8080).
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

    // Initialize tracing only for non-call and non-top commands
    // - call: keep output clean for unix tool usage (piping to jq, etc.)
    // - top: prevent logs from messing up the TUI (errors are shown in the UI instead)
    if !matches!(cli.command, Commands::Call(_) | Commands::Top(_)) {
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

            // Validate orchestrator URL if provided
            if let Some(orch_addr) = &args.orchestrator {
                validate_http_url(orch_addr, "orchestrator address")?;
                tracing::info!("Configured with orchestrator: {}", orch_addr);
            }

            // Create resource limits configuration
            let resource_limits = madrpc_server::ResourceLimits::new()
                .with_execution_timeout(std::time::Duration::from_millis(args.max_execution_time_ms));

            tracing::info!("Maximum execution time: {}ms", args.max_execution_time_ms);

            let node = if let Some(orch_addr) = &args.orchestrator {
                madrpc_server::Node::with_orchestrator_and_resource_limits(
                    std::path::PathBuf::from(&args.script),
                    orch_addr.clone(),
                    resource_limits,
                )?
            } else {
                madrpc_server::Node::with_resource_limits(
                    std::path::PathBuf::from(&args.script),
                    resource_limits,
                )?
            };
            tracing::info!("Node created successfully from script");

            let node_arc = std::sync::Arc::new(node);
            let mut server = madrpc_server::HttpServer::new(node_arc.clone());

            // Configure authentication if API key is provided
            if let Some(api_key) = &args.api_key {
                tracing::info!("API key authentication enabled");
                server = server.with_auth(madrpc_common::auth::AuthConfig::with_api_key(api_key));
            }

            // Configure rate limiting if specified
            if let Some(rps) = &args.rate_limit_rps {
                tracing::info!("Rate limiting enabled: {} requests per second", rps);
                server = server.with_rate_limit(madrpc_common::rate_limit::RateLimitConfig::per_second(*rps));
            }

            // Determine public URL for registration
            let public_url = if let Some(_orch_addr) = &args.orchestrator {
                // Priority: CLI flag > Env var > Auto-detect from bind
                let url = args.public_url
                    .or_else(|| std::env::var("MADRPC_PUBLIC_URL").ok())
                    .unwrap_or_else(|| format!("http://{}", args.bind));

                // Validate URL format
                validate_http_url(&url, "public URL")?;
                tracing::info!("Registering with public URL: {}", url);
                Some(url)
            } else {
                None
            };

            // Register with orchestrator if configured
            if let Some(url) = &public_url {
                if let Err(e) = node_arc.register_with_orchestrator(url.clone()).await {
                    tracing::error!("Failed to register with orchestrator: {}", e);
                    return Err(e.into());
                }
                tracing::info!("Successfully registered with orchestrator");
            }

            let addr: SocketAddr = args.bind.parse()
                .map_err(|e| anyhow::anyhow!("Invalid bind address {}: {}", args.bind, e))?;
            server.run(addr).await?;

            Ok(())
        }
        Commands::Orchestrator(args) => {
            tracing::info!("Starting MaDRPC orchestrator");
            tracing::info!("Binding to: {}", args.bind);
            tracing::info!("Nodes: {:?}", args.nodes);

            // Validate all node URLs
            for node_addr in &args.nodes {
                validate_http_url(node_addr, "node address")?;
            }

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
                madrpc_orchestrator::Orchestrator::with_config(args.nodes, config)?
            } else {
                let config = madrpc_orchestrator::HealthCheckConfig {
                    interval: std::time::Duration::from_secs(args.health_check_interval_secs),
                    timeout: std::time::Duration::from_millis(args.health_check_timeout_ms),
                    failure_threshold: args.health_check_failure_threshold,
                };
                madrpc_orchestrator::Orchestrator::with_config(args.nodes, config)?
            };

            tracing::info!("Orchestrator created with {} nodes", orch.node_count().await);

            let mut server = madrpc_orchestrator::HttpServer::new(std::sync::Arc::new(orch));

            // Configure authentication if API key is provided
            if let Some(api_key) = &args.api_key {
                tracing::info!("API key authentication enabled");
                server = server.with_auth(madrpc_common::auth::AuthConfig::with_api_key(api_key));
            }

            // Configure rate limiting if specified
            if let Some(rps) = &args.rate_limit_rps {
                tracing::info!("Rate limiting enabled: {} requests per second", rps);
                server = server.with_rate_limit(madrpc_common::rate_limit::RateLimitConfig::per_second(*rps));
            }

            let addr: SocketAddr = args.bind.parse()
                .map_err(|e| anyhow::anyhow!("Invalid bind address {}: {}", args.bind, e))?;
            server.run(addr).await?;

            Ok(())
        }
        Commands::Top(args) => {
            // Validate server address
            validate_http_url(&args.server_address, "server address")?;
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
    // Validate server address
    validate_http_url(&args.server_address, "server address")?;

    // Parse args JSON string
    let args_value: serde_json::Value = serde_json::from_str(&args.args)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in args: {}", e))?;

    // Create client and call method
    let client = madrpc_client::MadrpcClient::new(&args.server_address)?;
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
            Commands::Node(NodeArgs { script, bind, orchestrator, api_key, rate_limit_rps, max_execution_time_ms, public_url }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
                assert!(orchestrator.is_none());
                assert!(api_key.is_none());
                assert!(rate_limit_rps.is_none());
                assert_eq!(max_execution_time_ms, 30000); // default
                assert!(public_url.is_none());
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
            Commands::Node(NodeArgs { script, bind, orchestrator, api_key, rate_limit_rps, max_execution_time_ms, public_url }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
                assert_eq!(orchestrator, Some("127.0.0.1:8080".to_string()));
                assert!(api_key.is_none());
                assert!(rate_limit_rps.is_none());
                assert_eq!(max_execution_time_ms, 30000); // default
                assert!(public_url.is_none());
            }
            _ => panic!("Expected Node command"),
        }
    }

    #[test]
    fn test_cli_parse_node_with_api_key() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "node",
            "-s", "test.js",
            "-b", "0.0.0.0:9001",
            "--api-key", "my-secret-key",
        ]).unwrap();
        match args.command {
            Commands::Node(NodeArgs { script, bind, orchestrator, api_key, rate_limit_rps, max_execution_time_ms, public_url }) => {
                assert_eq!(script, "test.js");
                assert_eq!(bind, "0.0.0.0:9001");
                assert!(orchestrator.is_none());
                assert_eq!(api_key, Some("my-secret-key".to_string()));
                assert!(rate_limit_rps.is_none());
                assert_eq!(max_execution_time_ms, 30000); // default
                assert!(public_url.is_none());
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
            Commands::Orchestrator(OrchestratorArgs { bind, api_key, .. }) => {
                assert_eq!(bind, "0.0.0.0:9000");
                assert!(api_key.is_none());
            }
            _ => panic!("Expected Orchestrator command"),
        }
    }

    #[test]
    fn test_cli_parse_orchestrator_with_api_key() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "orchestrator",
            "--bind", "0.0.0.0:9000",
            "--api-key", "my-secret-key",
        ]).unwrap();
        match args.command {
            Commands::Orchestrator(OrchestratorArgs { bind, api_key, .. }) => {
                assert_eq!(bind, "0.0.0.0:9000");
                assert_eq!(api_key, Some("my-secret-key".to_string()));
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

    #[test]
    fn test_cli_parse_node_with_execution_timeout() {
        let args: Cli = Cli::from_args(&["madrpc"], &[
            "node",
            "-s", "test.js",
            "-b", "0.0.0.0:9001",
            "--max-execution-time-ms", "5000",
        ]).unwrap();
        match args.command {
            Commands::Node(NodeArgs { max_execution_time_ms, .. }) => {
                assert_eq!(max_execution_time_ms, 5000);
            }
            _ => panic!("Expected Node command"),
        }
    }
}
