# MaDRPC

A distributed RPC system in Rust that runs JavaScript functions across multiple nodes. Uses Boa as the JS engine and TCP for transport.

## Architecture

```
client -> orchestrator (round-robin) -> node 1, node 2, ...
                                      (Boa JS engine)
```

- **Orchestrator**: Simple round-robin load balancer, forwards requests to nodes
- **Node**: Executes JavaScript functions using Boa, thread-per-connection
- **Client**: Makes RPC calls with connection pooling

## Quick Start

```bash
# Build
cargo build --release

# Start orchestrator
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:8080 -n 127.0.0.1:9001

# Start node
cargo run --bin madrpc -- node -s script.js -b 0.0.0.0:9001
```

## Writing RPCs

Register functions in JavaScript:

```javascript
madrpc.register('add', (args) => {
    return { result: args.a + args.b };
});
```

## Client Usage

```rust
let client = MadrpcClient::new("127.0.0.1:8080").await?;
let result = client.call("add", json!({"a": 1, "b": 2})).await?;
```

## Workspace

```
crates/
  madrpc-common/       # Protocol, TCP transport
  madrpc-server/       # Node (thread-per-connection)
  madrpc-orchestrator/ # Load balancer
  madrpc-client/       # RPC client with pooling
  madrpc-metrics/      # Metrics for `top` command
  madrpc-cli/          # CLI entry point
```

## Examples

```bash
# Monte Carlo Pi estimation
cargo run -p monte-carlo-pi

# Monitor metrics
cargo run --bin madrpc -- top 127.0.0.1:8080
```
