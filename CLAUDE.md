# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## MaDRPC - Massively Distributed RPC

MaDRPC is a Rust-based distributed RPC system that enables massively parallel computation by executing JavaScript functions across multiple nodes. The system uses **Boa** as the JavaScript engine and **HTTP/JSON-RPC 2.0** for transport.

## Build and Run Commands

```bash
# Build the project
cargo build --release

# Start an orchestrator (load balancer)
cargo run --bin madrpc -- orchestrator \
  -b 0.0.0.0:8080 \
  -n http://127.0.0.1:9001 \
  -n http://127.0.0.1:9002

# Start a compute node
cargo run --bin madrpc -- node \
  -s examples/monte-carlo-pi/scripts/pi.js \
  -b 0.0.0.0:9001

# Monitor with real-time metrics TUI
cargo run --bin madrpc -- top http://127.0.0.1:8080

# Run examples
cargo run -p monte-carlo-pi
cargo run -p simple-test
```

## Project Roadmap

**Location**: `@reference/ROADMAP.md` (optional - only used if the file exists)

> **Note**: This section provides guidance for using a ROADMAP.md file if one exists in the project. If no ROADMAP.md file is present, you can safely ignore this section.

### What is the Roadmap?

When present, the **ROADMAP.md** file serves as a **single entry point for planning and task management**. It typically describes:

1. **Overall Development Process** - Workflow principles, phase structure, and how to approach different types of work
2. **High-Level Task Overviews** - Each task organized by phase with bullet points describing:
   - Affected crate and file locations
   - Impact and rationale
   - Estimated effort
   - Implementation steps
   - Testing requirements

### When to Consult the Roadmap (if it exists)

**BEFORE starting any work:**
- Check if the task is already tracked in the roadmap
- Understand the current phase and priorities
- Review related tasks that may affect your approach
- Identify dependencies between tasks

**When planning new features:**
- Consult the appropriate phase for context
- Ensure the feature aligns with current priorities
- Check for conflicting or related work
- Add new tasks to the appropriate phase

**When choosing what to work on:**
- Start with Phase 1 (critical bugs) if not complete
- Move to Phase 2 (high priority) after Phase 1
- Check the "Quick Reference" section for current blockers
- Follow the "Task Prioritization Guidelines"

### When to Update the Roadmap

**UPDATE the roadmap when:**
- A task is completed (mark as completed with date)
- New issues are discovered (add to appropriate phase)
- Priorities change (update task order or phase)
- New features are planned (add to appropriate phase)
- Tasks become blocked or unblocked (update blockers section)
- Estimates are found to be inaccurate (update with new estimates)

**DO NOT update the roadmap when:**
- Making atomic commits (update after the full task is complete)
- Running tests or doing routine maintenance
- Making documentation updates to existing features
- Fixing minor typos or formatting issues

### Roadmap Structure (when present)

```
reference/ROADMAP.md
├── Overview                    # Project status and purpose
├── Development Process         # Workflow principles and phases
├── Phase 1 (Immediate)         # Critical bugs - Week 1
├── Phase 2 (Short-term)        # High priority - Weeks 2-3
├── Phase 3 (Medium-term)       # Security & performance - Month 2
├── Phase 4 (Long-term)         # Polish & enhancements - Month 3+
├── Cross-Cutting Initiatives   # Testing, DX, security
├── Task Prioritization         # Guidelines for choosing work
└── Quick Reference             # Current priorities and blockers
```

### Key Principles (when using a roadmap)

1. **Roadmap First** - Always check the roadmap before starting new work
2. **One Task at a Time** - Complete a task from start to finish before starting another
3. **Update as You Go** - Mark tasks complete immediately after finishing
4. **Add Context** - When adding new tasks, include all required information
5. **Keep it Living** - The roadmap evolves with the project

### Related Files (when present)

- [`REPORT.md`](REPORT.md) - Detailed quality analysis findings (source of roadmap tasks)
- [`CLAUDE.md`](CLAUDE.md) - This file (development practices and architecture)
- [`reference/ROADMAP.md`](reference/ROADMAP.md) - Task planning and prioritization (if it exists)

## Architecture Overview

The system consists of three main components that communicate via HTTP:

1. **Nodes** (`madrpc-server`) - Execute JavaScript functions using Boa
2. **Orchestrator** (`madrpc-orchestrator`) - Round-robin load balancer that forwards requests
3. **Client** (`madrpc-client`) - Makes RPC calls to the orchestrator

### Key Design Patterns

- **"Stupid Forwarder" Orchestrator**: The orchestrator does no JavaScript execution - it only forwards requests to nodes using round-robin load balancing
- **HTTP/JSON-RPC 2.0**: All communication uses standard JSON-RPC 2.0 over HTTP POST
- **Connection Keep-Alive**: Hyper's HTTP/1.1 keep-alive handles connection pooling automatically
- **Async/Await**: Orchestrator and client use tokio async runtime for efficient concurrent request handling

### Data Flow

1. Client makes HTTP POST with JSON-RPC request to orchestrator
2. Orchestrator selects next node via round-robin (`LoadBalancer::next_node()`)
3. Request forwarded to selected compute node via HTTP
4. Node creates a fresh Boa Context and executes function
5. Result returned through orchestrator to client as JSON-RPC response

### Wire Protocol

- **Transport**: HTTP/1.1 with JSON-RPC 2.0
- **Serialization**: JSON
- **Message Format**: JSON-RPC 2.0 specification (`{"jsonrpc":"2.0","method":"...","params":{...},"id":...}`)
- **Content-Type**: `application/json`

## Directory Structure

```
madrpc-2/
├── crates/                          # Core library crates
│   ├── madrpc-common/               # Shared protocol and transport
│   ├── madrpc-server/              # Node implementation (JavaScript execution)
│   ├── madrpc-orchestrator/       # Load balancer and request forwarder
│   ├── madrpc-client/              # RPC client with connection pooling
│   ├── madrpc-metrics/            # Metrics collection infrastructure
│   └── madrpc-cli/               # CLI entry point
├── examples/                      # Example applications
│   ├── monte-carlo-pi/           # Monte Carlo Pi estimation
│   └── simple-test/              # Basic RPC test
├── tests/                        # Integration tests
└── Cargo.toml                    # Workspace configuration
```

## Crate-by-Crate Guide

### `crates/madrpc-common` - Protocol and Transport Layer

**Purpose**: Core protocol definitions and HTTP transport layer shared across all components

**Where to find what**:
- `src/lib.rs` - Public API exports
- `src/protocol/` - RPC protocol definitions
  - `jsonrpc.rs` - JSON-RPC 2.0 request/response types
  - `requests.rs` - Legacy request types (deprecated)
  - `responses.rs` - Legacy response types (deprecated)
  - `error.rs` - `MadrpcError` enum with retryable error classification
- `src/transport/` - HTTP transport
  - `http.rs` - HTTP transport using hyper/axum

**Key Types**:
```rust
pub type RequestId = u64;
pub type MethodName = String;
pub type RpcArgs = serde_json::Value;
pub type RpcResult = serde_json::Value;

pub struct Request { /* id, method, args, timeout_ms, idempotency_key */ }
pub struct Response { /* id, result, error, success */ }
pub enum MadrpcError { /* Transport, JsonSerialization, Timeout, etc. */ }
```

### `crates/madrpc-server` - Node Implementation

**Purpose**: Execute JavaScript functions using Boa JavaScript engine

**Where to find what**:
- `src/lib.rs` - Public API and Node struct
- `src/node.rs` - Main `Node` struct that handles RPC requests
- `src/runtime/` - Boa JavaScript integration
  - `context.rs` - Thread-safe Boa context wrapper with distributed RPC support
  - `bindings.rs` - JavaScript bindings (`madrpc.register`, `madrpc.call`)
  - `conversions.rs` - JSON ↔ JavaScript value conversions
  - `job_executor.rs` - Tokio integration for async JavaScript
- `src/http_router.rs` - HTTP router with axum
- `src/http_server.rs` - HTTP server using hyper

**Critical Implementation Details**:
- **Thread Safety**: Each request creates a fresh Boa Context to enable true parallelism
- **Script Caching**: Nodes cache script source to avoid file I/O, but parse in each Context due to Boa's string interner

### `crates/madrpc-orchestrator` - Load Balancer

**Purpose**: Round-robin load balancer with circuit breaker and health checking

**Where to find what**:
- `src/lib.rs` - Public API exports
- `src/orchestrator.rs` - Main orchestrator implementation
- `src/load_balancer.rs` - Round-robin selection with circuit breaker
- `src/node.rs` - Node state management with health tracking
- `src/health_checker.rs` - Periodic health checks
- `src/http_router.rs` - HTTP router with fallback forwarding
- `src/http_server.rs` - HTTP server using axum

**Circuit Breaker**: Exponential backoff with configurable failure threshold, timeout, and multiplier.

### `crates/madrpc-client` - RPC Client

**Purpose**: Client for making RPC calls with automatic retry logic

**Where to find what**:
- `src/lib.rs` - Public API exports
- `src/client.rs` - Main `MadrpcClient` with retry logic and HTTP client

**Retry Logic**: Exponential backoff with jitter for transient errors (network issues, timeouts).

### `crates/madrpc-metrics` - Metrics Infrastructure

**Purpose**: Metrics collection for nodes and orchestrators

**Where to find what**:
- `src/lib.rs` - Public API exports
- `src/collector.rs` - `MetricsCollector` trait and implementations
- `src/registry.rs` - Thread-safe metrics storage
- `src/snapshot.rs` - Metrics snapshot types

**Built-in Endpoints**: `_metrics` and `_info` for monitoring.

### `crates/madrpc-cli` - Command Line Interface

**Purpose**: CLI entry point with commands for all components

**Where to find what**:
- `src/main.rs` - CLI argument parsing and command dispatch
- `src/top.rs` - Real-time metrics TUI using ratatui
- `src/commands/` - Individual command implementations
  - `node.rs` - Node command implementation
  - `orchestrator.rs` - Orchestrator command implementation
  - `call.rs` - RPC call command implementation

## JavaScript Integration

### Script Registration Pattern

Scripts register functions using the global `madrpc.register(name, function)`:

```javascript
madrpc.register('monte_carlo_sample', (args) => {
    const samples = args.samples || 1000000;
    const seed = args.seed || 0;
    // ... computation
    return { inside: inside, total: samples };
});
```

### Distributed RPC Calls

Nodes can make distributed RPC calls using `madrpc.call()`:

```javascript
madrpc.register('aggregate', async (args) => {
    const promises = [];
    for (let i = 0; i < args.numNodes; i++) {
        promises.push(madrpc.call('monte_carlo_sample', {
            samples: args.samplesPerNode,
            seed: i
        }));
    }
    const results = await Promise.all(promises);
    // ... aggregation
    return { piEstimate: 4 * totalInside / totalSamples };
});
```

**Where to find JavaScript integration**:
- `crates/madrpc-server/src/runtime/bindings.rs` - JavaScript function implementations
- `crates/madrpc-server/src/runtime/context.rs` - Boa context management
- `examples/` - JavaScript script examples

## Thread Model

### Orchestrator (Async)
- Uses tokio async runtime with multi-threaded scheduler
- `HttpServer` uses axum for HTTP handling
- Single thread can handle many connections efficiently
- No JavaScript execution (just forwarding)

### Nodes (Async)
- `HttpServer` uses hyper for HTTP handling
- Each request spawns a tokio task
- Each request gets fresh Boa Context
- True parallelism with no shared JavaScript state

## Error Handling

### Error Classification

**Retryable Errors** (transient):
- Network issues, timeouts, connection failures
- Node unavailable

**Non-Retryable Errors** (permanent):
- Invalid requests, JavaScript execution errors
- Invalid responses, all nodes failed

**Where to find**: `crates/madrpc-common/src/protocol/error.rs`

### Circuit Breaker Implementation

- **Closed**: Normal operation
- **Open**: Fail fast after consecutive failures
- **Half-Open**: Test recovery with exponential backoff

**Where to find**: `crates/madrpc-orchestrator/src/load_balancer.rs`

## Examples

### Monte Carlo Pi Estimation
- **Location**: `examples/monte-carlo-pi/`
- **Purpose**: Demonstrates distributed parallel computation
- **Features**: JavaScript orchestrates 50 parallel RPC calls using LCG for reproducible random numbers

### Simple Test
- **Location**: `examples/simple-test/`
- **Purpose**: Basic RPC call example
- **Use**: Good starting point for understanding the system

## Key Dependencies

### Core Dependencies
- **tokio** - Async runtime (orchestrator, client)
- **boa_engine** - JavaScript engine (nodes)
- **serde_json** - JSON serialization
- **thiserror** - Error handling
- **tracing** - Structured logging

### CLI Dependencies
- **argh** - Command line parsing
- **ratatui** - Terminal UI for monitoring
- **crossterm** - Cross-platform terminal handling

## Important Implementation Notes

- **Boa Context Threading**: Boa Context has thread-local state and must be accessed from the same thread it was created on. Each request creates its own context.
- **Fresh Context Per Request**: Each request gets a fresh Boa Context to enable true parallelism without shared state.
- **HTTP/JSON-RPC**: Uses standard JSON-RPC 2.0 over HTTP POST for all communication.
- **Async/Await**: All HTTP servers use tokio async runtime for efficient concurrent request handling.
- **Metrics**: Built-in metrics collection on nodes/orchestrator via `madrpc-metrics` - see `top` command for TUI monitoring

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
