# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## MaDRPC - Massively Distributed RPC

MaDRPC is a Rust-based distributed RPC system that enables massively parallel computation by executing JavaScript functions across multiple nodes. The system uses **Boa** as the JavaScript engine and **TCP** for transport.

## Build and Run Commands

```bash
# Build the project
cargo build --release

# Start an orchestrator (load balancer)
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:8080 -n 127.0.0.1:9001 -n 127.0.0.1:9002

# Start a compute node
cargo run --bin madrpc -- node -s examples/monte-carlo-pi/scripts/pi.js -b 0.0.0.0:9001 --pool-size 4

# Monitor with real-time metrics TUI
cargo run --bin madrpc -- top 127.0.0.1:8080

# Run examples
cargo run -p monte-carlo-pi
cargo run -p simple-test
```

## Architecture

The system consists of three main components that communicate via TCP:

1. **Nodes** (`madrpc-server`) - Execute JavaScript functions using Boa
2. **Orchestrator** (`madrpc-orchestrator`) - Round-robin load balancer that forwards requests
3. **Client** (`madrpc-client`) - Makes RPC calls to the orchestrator

### Key Design Patterns

- **"Stupid Forwarder" Orchestrator**: The orchestrator does no JavaScript execution - it only forwards requests to nodes using round-robin load balancing
- **Thread-Per-Connection Nodes**: Each node accepts TCP connections and spawns a dedicated worker thread for each connection, limited by a semaphore (default: `num_cpus * 2`)
- **Connection Pooling**: Client maintains connection pools for performance
- **Keep-Alive**: Each TCP connection processes multiple requests until closed

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
3. Request forwarded to selected compute node via TCP
4. Node creates a fresh Boa Context on the worker thread and executes function
5. Result returned through orchestrator to client

## Important Implementation Notes

- **Boa Context Threading**: Boa Context has thread-local state and must be accessed from the same thread it was created on. Each worker thread creates its own context per request.
- **No Context Pool**: Previous QuickJS approach used a context pool, but this was removed due to thread-safety issues. Each request now gets a fresh Boa Context.
- **TCP Transport**: Uses `postcard` for binary serialization over plain TCP sockets with 4-byte length prefix (big-endian).
- **Thread-Per-Connection**: Nodes use `TcpServerThreaded` which spawns OS threads (not async tasks) for each connection, limited by a custom `StdSemaphore` using `AtomicUsize` and `Condvar`.
- **Orchestrator is Async**: The orchestrator uses `TcpServer` (tokio async) since it doesn't use Boa and handles many concurrent connections efficiently.
- **Metrics**: Built-in metrics collection on nodes/orchestrator via `madrpc-metrics` - see `top` command for TUI monitoring

## Workspace Structure

- `crates/madrpc-common` - Protocol definitions and TCP transport
- `crates/madrpc-server` - Node implementation with thread-per-connection
- `crates/madrpc-orchestrator` - Load balancer and request forwarder
- `crates/madrpc-client` - RPC client with connection pooling
- `crates/madrpc-metrics` - Metrics collection infrastructure
- `crates/madrpc-cli` - CLI entry point with `node`, `orchestrator`, and `top` commands
- `examples/` - Example applications demonstrating usage
- `tests/` - Integration tests
