# MaDRPC - Massively Distributed RPC

MaDRPC is a Rust-based distributed RPC system that enables massively parallel computation by executing JavaScript functions across multiple nodes. It provides a simple yet powerful way to distribute computational workloads across a cluster of machines.

## Features

- **Distributed Execution**: Run JavaScript functions across multiple nodes in parallel
- **Automatic Load Balancing**: Built-in orchestrator with round-robin request distribution
- **Connection Pooling**: Efficient TCP connection management for high throughput
- **Real-time Monitoring**: TUI-based monitoring with the `top` command
- **Simple JavaScript API**: Register functions with a single `madrpc.register()` call
- **Type-Safe Rust Client**: Strongly typed Rust client with serde JSON integration
- **Thread-Per-Connection Architecture**: Each connection gets a dedicated thread with its own JavaScript context

## Quick Start

### 1. Build the Project

```bash
cargo build --release
```

### 2. Start an Orchestrator

The orchestrator acts as a load balancer, distributing requests across all available nodes.

```bash
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:8080 -n 127.0.0.1:9001 -n 127.0.0.1:9002
```

Options:
- `-b, --bind`: Address to bind to (default: `0.0.0.0:8080`)
- `-n, --node`: Node addresses (can be specified multiple times)

### 3. Start Compute Nodes

Each node runs a JavaScript file and accepts connections from the orchestrator.

```bash
cargo run --bin madrpc -- node -s scripts/pi.js -b 0.0.0.0:9001 --pool-size 4
```

Options:
- `-s, --script`: JavaScript file to load (required)
- `-b, --bind`: Address to bind to (default: `0.0.0.0:0`)
- `--pool-size`: Number of worker threads (default: `num_cpus * 2`)

### 4. Make RPC Calls from Rust

```rust
use madrpc_client::MadrpcClient;
use madrpc_client::PoolConfig;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Connect to the orchestrator with default pool (max 10 connections)
    let client = MadrpcClient::new("127.0.0.1:8080").await?;

    // Or with custom pool configuration
    let client = MadrpcClient::with_config(
        "127.0.0.1:8080",
        PoolConfig { max_connections: 20 }
    ).await?;

    // Call a registered function
    let result = client.call("function_name", json!({
        "param1": "value1",
        "param2": 42
    })).await?;

    println!("Result: {}", result);
    Ok(())
}
```

The client automatically manages connection pooling - connections are reused across requests for better performance.

## Architecture

MaDRPC consists of three main components:

```
+---------+     TCP      +--------------+     TCP      +-------+
| Client  | ----------> | Orchestrator | ----------> | Node  |
+---------+             +--------------+             +-------+
                                                          |
                                                       Boa JS
                                                       Engine
```

### Components

#### 1. Client (`madrpc-client`)
- Makes RPC calls to the orchestrator
- Uses connection pooling to reuse TCP connections across requests
- Configurable pool size (default: 10 connections per address)
- Async/await API with tokio
- Efficient for parallel requests with shared connections

#### 2. Orchestrator (`madrpc-orchestrator`)
- "Stupid forwarder" - does no JavaScript execution
- Distributes requests via round-robin load balancing
- Handles many concurrent connections efficiently (async)

#### 3. Node (`madrpc-server`)
- Executes JavaScript functions using Boa engine
- Thread-per-connection architecture
- Each worker thread gets its own Boa Context
- Limited by semaphore to prevent resource exhaustion

### Data Flow

1. Client creates connection pool (or uses shared pool if cloned)
2. Client makes RPC call: `client.call("function", args)`
3. Client acquires connection from pool (or creates new one if needed)
4. Orchestrator selects next node via round-robin
5. Request forwarded to selected node via TCP
6. Node creates fresh Boa Context and executes function
7. Result returned through orchestrator to client
8. Client releases connection back to pool for reuse

### Wire Protocol

- Transport: TCP
- Serialization: postcard (binary format)
- Message format: `[4-byte length (u32 BE)] + [postcard data]`
- Keep-alive: Each connection processes multiple requests

## Writing RPCs

MaDRPC uses JavaScript for RPC implementations. Each node loads a JavaScript file that registers functions using the `madrpc.register()` API.

### Basic RPC Function

```javascript
// Simple addition function
madrpc.register('add', (args) => {
    const a = args.a || 0;
    const b = args.b || 0;
    return { result: a + b };
});
```

### Advanced Example: Monte Carlo Pi Estimation

```javascript
'use strict';

// Linear Congruential Generator for pseudo-random numbers
class LCG {
    constructor(seed) {
        this.state = seed;
    }

    // Returns a float between 0 and 1
    next() {
        this.state = (this.state * 1103515245 + 12345) & 0x7fffffff;
        return this.state / 0x7fffffff;
    }
}

// Monte Carlo sampling for Pi estimation
// Generates random points and counts how many fall inside the unit circle
madrpc.register('monte_carlo_sample', (args) => {
    const samples = args.samples || 1000000;
    const seed = args.seed || 0;

    const rng = new LCG(seed);
    let inside = 0;

    for (let i = 0; i < samples; i++) {
        const x = rng.next();
        const y = rng.next();

        // Check if point is inside unit circle
        if (x * x + y * y <= 1) {
            inside++;
        }
    }

    return {
        inside: inside,
        total: samples
    };
});
```

### RPC Guidelines

- **Arguments**: Access via `args.paramName` (always an object)
- **Return**: Always return an object (serialized as JSON)
- **Errors**: Throw exceptions to return errors to client
- **State**: Each request gets a fresh context - no persistent state between requests
- **Classes**: You can define classes and use them in your RPCs

## Examples

### Monte Carlo Pi Estimation

This example demonstrates parallel computation by estimating Pi using Monte Carlo methods across multiple nodes.

```bash
# Terminal 1: Start orchestrator
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:8080 -n 127.0.0.1:9001

# Terminal 2: Start node with pi.js script
cargo run --bin madrpc -- node -s examples/monte-carlo-pi/scripts/pi.js -b 0.0.0.0:9001

# Terminal 3: Run the example
cargo run -p monte-carlo-pi
```

The example spawns 50 parallel requests, distributes them across nodes, and aggregates results:

```
Computing Pi using Monte Carlo with 10000000 samples...
Using 50 nodes, 200000 samples per node
Node 0: 157079 inside out of 200000 samples
Node 1: 157081 inside out of 200000 samples
...

=== Results ===
Total inside: 7853981
Total samples: 10000000
Pi estimate:  3.1415924000
Actual Pi:     3.1415926536
Error:         0.0000002616
```

### Simple Test

A minimal example showing basic RPC usage:

```bash
# Terminal 1: Start orchestrator
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:18084 -n 127.0.0.1:9001

# Terminal 2: Start node
cargo run --bin madrpc -- node -s examples/monte-carlo-pi/scripts/pi.js -b 0.0.0.0:9001

# Terminal 3: Run simple test
cargo run -p simple-test
```

### Monitoring with Top

Watch real-time metrics from your orchestrator:

```bash
cargo run --bin madrpc -- top 127.0.0.1:8080
```

Options:
- `-i, --interval`: Refresh interval in milliseconds (default: 250)

## Implementation Notes

### Boa Context Threading

Boa JavaScript contexts have thread-local state and must be accessed from the same thread they were created on. MaDRPC handles this by:

1. Using a thread-per-connection architecture for nodes
2. Each worker thread creates its own Boa Context per request
3. No context pooling - each request gets a fresh context

### Thread Limits

Nodes limit concurrent connections using a semaphore:
- Default: `num_cpus * 2`
- Configurable via `--pool-size` flag
- Prevents resource exhaustion from too many worker threads

### TCP vs QUIC

MaDRPC uses plain TCP instead of QUIC for simplicity and to avoid the complexity of TLS certificate management. The wire protocol uses postcard for efficient binary serialization.

## Workspace Structure

```
madrpc-2/
├── crates/
│   ├── madrpc-common/       # Protocol definitions and TCP transport
│   ├── madrpc-server/       # Node implementation
│   ├── madrpc-orchestrator/ # Load balancer
│   ├── madrpc-client/       # RPC client
│   ├── madrpc-metrics/      # Metrics collection
│   └── madrpc-cli/          # CLI entry point
├── examples/
│   ├── monte-carlo-pi/      # Monte Carlo Pi estimation
│   └── simple-test/         # Basic usage example
└── tests/                   # Integration tests
```

## License

MIT
