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
- **TCP Transport**: Uses JSON for serialization over plain TCP sockets with 4-byte length prefix (big-endian).
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

## Development Practices

### Task Breakdown and Context Management

**CRITICAL**: Always split work into small, well-separated tasks and use subagents for each task. This keeps context windows clean and focused.

When working on complex features:

1. **Use the Task tool with subagent_type='Explore'** for codebase exploration
   - Finding files by patterns
   - Searching for code patterns
   - Understanding architecture

2. **Break down implementation into atomic tasks** such as:
   - "Add new RPC method to protocol"
   - "Update error handling for new case"
   - "Write unit tests for load balancer"
   - "Add integration test for new feature"

3. **Use specialized subagents** for each distinct task:
   - `general-purpose` - For multi-step implementation tasks
   - `Explore` - For codebase exploration and research
   - `Plan` - For designing implementation strategies

Example workflow:
```bash
# Step 1: Explore relevant code
# Step 2: Design the approach
# Step 3: Implement feature X
# Step 4: Write tests for feature X
# Step 5: Implement feature Y
# Step 6: Write tests for feature Y
```

### Git Commit Practices

**CRITICAL**: ALWAYS make small, atomic git commits. Never batch unrelated changes.

- Commit after each logical unit of work is complete
- Use semantic commit messages (lowercase, except proper nouns)
- Examples: `fix(server): handle connection timeout`, `feat(client): add retry logic`
- NEVER use `git add -A` or `git add .`
- Always add specific files: `git add crates/madrpc-server/src/node.rs`

Good commit pattern:
```bash
# Make a small change
git add crates/madrpc-server/src/node.rs
git commit -m "fix(node): handle empty script path"

# Make another small change
git add crates/madrpc-common/src/protocol/error.rs
git commit -m "feat(protocol): add timeout error variant"
```

### Testing Practices

**CRITICAL**: ALWAYS write unit tests for new functionality.

#### Unit Tests

Place unit tests in the same module using `#[cfg(test)]`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_creation() {
        let req = Request::new("test_method", json!({"arg": 42}));
        assert_eq!(req.method, "test_method");
    }

    #[test]
    fn test_with_timeout() {
        let req = Request::new("test", json!({})).with_timeout(5000);
        assert_eq!(req.timeout_ms, Some(5000));
    }
}
```

For larger test suites, create a separate `tests.rs` module:
```rust
// In protocol/mod.rs
#[cfg(test)]
mod tests;
```

#### Integration Tests

Place integration tests in `tests/` directory:
- Use clear section headers to organize tests
- Create helper functions for test setup
- Focus on component integration over full network tests

Example structure:
```rust
// ============================================================================
// Protocol Layer Tests
// ============================================================================

#[tokio::test]
async fn test_protocol_request_response_flow() {
    // ...
}

// ============================================================================
// Load Balancer Tests
// ============================================================================
```

#### Test Organization

- **Unit tests**: Test individual functions and structs in-place
- **Integration tests**: Test component interactions in `tests/`
- **Helper functions**: Create reusable test utilities (e.g., `create_test_script()`)
- **Use descriptive names**: `test_load_balancer_round_robin`, not `test_lb`

### Code Organization

#### Module Structure

Keep modules focused and cohesive:

```rust
// lib.rs - public API
pub mod protocol;
pub mod transport;
pub use protocol::*;

// protocol/mod.rs - internal organization
pub mod error;
pub mod requests;
pub mod responses;
#[cfg(test)]
mod tests;
```

#### Type Aliases

Use type aliases for clarity:

```rust
pub type RequestId = u64;
pub type MethodName = String;
pub type RpcArgs = serde_json::Value;
pub type Result<T> = std::result::Result<T, MadrpcError>;
```

#### Error Handling

Use `thiserror` for comprehensive error types:

```rust
#[derive(Error, Debug)]
pub enum MadrpcError {
    #[error("Transport error: {0}")]
    Transport(String),
    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),
    #[error("Request timeout after {0}ms")]
    Timeout(u64),
}
```

### Code Style Guidelines

- **Write simple, efficient, elegant code** - favor clarity over cleverness
- **Document public APIs** with `///` doc comments
- **Use `tracing` for logging** - not `println!` or `log`
- **Prefer `Arc<Mutex<T>>`** for shared state across threads
- **Use `async/await` consistently** with tokio runtime
- **Follow Rust naming conventions**: `snake_case` for functions/variables, `PascalCase` for types

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p madrpc-server

# Run tests with output
cargo test -- --nocapture

# Run integration tests only
cargo test --test integration_test
```

### Adding New Features

When adding new features:

1. **Explore first**: Use `Explore` agent to understand relevant code
2. **Plan**: Use `Plan` agent to design the approach
3. **Implement**: Write the feature code
4. **Test**: Write comprehensive unit tests
5. **Commit**: Make atomic commit for the feature
6. **Repeat**: For each logical sub-feature

Example for adding a new RPC method:
```
1. Explore protocol/request handling code
2. Plan the implementation approach
3. Update protocol types (add new method enum)
4. Update error handling if needed
5. Write unit tests for protocol changes
6. Commit: "feat(protocol): add new method type"
7. Update node to handle new method
8. Write integration tests
9. Commit: "feat(node): implement new method handler"
```
