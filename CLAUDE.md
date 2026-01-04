# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## MaDRPC - Massively Distributed RPC

MaDRPC is a Rust-based distributed RPC system that enables massively parallel computation by executing JavaScript functions across multiple nodes. The system uses QuickJS as the JavaScript engine and QUIC for secure, high-performance transport.

## Build and Run Commands

```bash
# Build the project
cargo build --release

# Start an orchestrator (load balancer)
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:8080 -n 0.0.0.0:9001 -n 0.0.0.0:9002

# Start a compute node
cargo run --bin madrpc -- node -s examples/monte-carlo-pi/scripts/pi.js -b 0.0.0.0:9001 --pool-size 4

# Monitor with real-time metrics TUI
cargo run --bin madrpc -- top 127.0.0.1:8080

# Run examples
cargo run -p monte-carlo-pi
cargo run -p simple-test
```

## Architecture

The system consists of three main components that communicate via QUIC:

1. **Nodes** (`madrpc-server`) - Execute JavaScript functions using QuickJS
2. **Orchestrator** (`madrpc-orchestrator`) - Round-robin load balancer that forwards requests
3. **Client** (`madrpc-client`) - Makes RPC calls to the orchestrator

### Key Design Patterns

- **"Stupid Forwarder" Orchestrator**: The orchestrator does no JavaScript execution - it only forwards requests to nodes using round-robin load balancing
- **Context Pool**: Each node maintains a pool of QuickJS contexts for parallel execution (pool size defaults to `num_cpus` but is configurable via CLI)
- **Connection Pooling**: Client maintains connection pools for performance

### JavaScript Scripts

Scripts register functions using the global `madrpc.register(name, function)`:

```javascript
madrpc.register('monte_carlo_sample', (args) => {
    const samples = args.samples || 1000000;
    const seed = args.seed || 0;
    // ... computation
    return { inside: inside, total: samples };
});
```

### Data Flow

1. Client makes RPC call to orchestrator
2. Orchestrator selects next node via round-robin (`LoadBalancer::next_node()`)
3. Request forwarded to selected compute node via QUIC
4. Node acquires QuickJS context from pool and executes function
5. Result returned through orchestrator to client

## Important Implementation Notes

- **QuickJS Threading**: QuickJS contexts are NOT thread-safe. The context pool uses `Arc<Mutex<>>` wrapping and `tokio::task::spawn_blocking` for safe execution
- **Context Pool Auto-Release**: `PooledContext` uses RAII - when dropped, it automatically returns the context to the pool via `Drop` trait
- **QUIC Transport**: Uses `postcard` for binary serialization and `rustls` for TLS encryption
- **Metrics**: Built-in metrics collection on nodes/orchestrator via `madrpc-metrics` - see `top` command for TUI monitoring

## Workspace Structure

- `crates/madrpc-common` - Protocol definitions and QUIC transport
- `crates/madrpc-server` - Node implementation with QuickJS context pool
- `crates/madrpc-orchestrator` - Load balancer and request forwarder
- `crates/madrpc-client` - RPC client with connection pooling
- `crates/madrpc-metrics` - Metrics collection infrastructure
- `crates/madrpc-cli` - CLI entry point with `node`, `orchestrator`, and `top` commands
- `examples/` - Example applications demonstrating usage
- `tests/` - Integration tests
