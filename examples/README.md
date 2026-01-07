# MaDRPC Examples

This directory contains JavaScript examples demonstrating various features of the MaDRPC (Massively Distributed RPC) system.

## Overview

Each example is a standalone JavaScript file that can be loaded into a MaDRPC node. The examples demonstrate different aspects of distributed RPC programming, from basic operations to complex parallel computations.

## Running the Examples

### 1. Start an Orchestrator

First, start the orchestrator (load balancer):

```bash
cargo run --bin madrpc -- orchestrator -b 0.0.0.0:8080
```

### 2. Start Compute Nodes

Start one or more compute nodes with the example script:

```bash
# Terminal 1: Node with basic operations
cargo run --bin madrpc -- node -s examples/basic-operations.js -b 0.0.0.0:9001 --pool-size 4

# Terminal 2: Node with Monte Carlo example
cargo run --bin madrpc -- node -s examples/monte-carlo-pi.js -b 0.0.0.0:9002 --pool-size 4
```

### 3. Make RPC Calls

Use the CLI to make RPC calls:

```bash
# Basic operations
cargo run --bin madrpc -- call 127.0.0.1:8080 add '{"a": 10, "b": 5}'
cargo run --bin madrpc -- call 127.0.0.1:8080 concatenate '{"str1": "Hello", "str2": "World", "separator": " "}'

# Monte Carlo Pi estimation
cargo run --bin madrpc -- call 127.0.0.1:8080 aggregate '{"numNodes": 10, "samplesPerNode": 100000}'
cargo run --bin madrpc -- call 127.0.0.1:8080 estimate_single '{"totalSamples": 1000000}'
```

### 4. Monitor Performance (Optional)

Use the built-in TUI to monitor real-time metrics:

```bash
cargo run --bin madrpc -- top 127.0.0.1:8080
```

## Examples

### basic-operations.js

**Purpose**: Demonstrates fundamental RPC operations and basic request/response patterns.

**Features demonstrated**:
- Number operations: `add`, `subtract`, `multiply`, `divide`
- String operations: `concatenate`, `reverse`, `uppercase`, `lowercase`
- JSON operations: `merge`, `deep_clone`, `pick`
- Error handling (division by zero)
- Aggregation patterns (`math_pipeline`, `word_count`)

**Example calls**:

```bash
# Number operations
cargo run --bin madrpc -- call 127.0.0.1:8080 add '{"a": 10, "b": 5}'
cargo run --bin madrpc -- call 127.0.0.1:8080 divide '{"a": 20, "b": 3}'

# String operations
cargo run --bin madrpc -- call 127.0.0.1:8080 reverse '{"str": "hello"}'
cargo run --bin madrpc -- call 127.0.0.1:8080 concatenate '{"str1": "Hello", "str2": "World", "separator": " "}'

# JSON operations
cargo run --bin madrpc -- call 127.0.0.1:8080 merge '{"obj1": {"a": 1}, "obj2": {"b": 2}}'
cargo run --bin madrpc -- call 127.0.0.1:8080 pick '{"obj": {"name": "Alice", "age": 30, "city": "NYC"}, "fields": ["name", "age"]}'

# Aggregation
cargo run --bin madrpc -- call 127.0.0.1:8080 math_pipeline '{"initial": 10, "operations": [{"operation": "multiply", "value": 2}, {"operation": "add", "value": 5}]}'
cargo run --bin madrpc -- call 127.0.0.1:8080 word_count '{"text": "hello world hello rust world"}'
```

**Best for**: Beginners learning the basics of MaDRPC, understanding the `madrpc.register()` API, and simple request/response patterns.

---

### monte-carlo-pi.js

**Purpose**: Demonstrates distributed parallel computation for Monte Carlo Pi estimation.

**Features demonstrated**:
- Distributed computation across multiple nodes
- Async RPC calls using `madrpc.call()`
- Parallel execution with `Promise.all()`
- Result aggregation from multiple sources
- Reproducible random number generation (LCG)
- Single-node vs distributed computation comparison

**Registered functions**:
- `monte_carlo_sample`: Performs Monte Carlo sampling on a single node
- `aggregate`: Orchestrates parallel sampling across multiple nodes
- `estimate_single`: Single-node estimation for performance comparison

**Example calls**:

```bash
# Distributed estimation (50 parallel calls)
cargo run --bin madrpc -- call 127.0.0.1:8080 aggregate '{"numNodes": 50, "samplesPerNode": 200000}'

# Single-node estimation (for comparison)
cargo run --bin madrpc -- call 127.0.0.1:8080 estimate_single '{"totalSamples": 10000000}'

# Small scale test
cargo run --bin madrpc -- call 127.0.0.1:8080 aggregate '{"numNodes": 10, "samplesPerNode": 100000}'
```

**Expected output**: The result will include:
- `totalInside`: Number of points inside the unit circle
- `totalSamples`: Total number of points sampled
- `piEstimate`: Estimated value of Pi (should be close to 3.14159...)

**Best for**: Learning about distributed computation, parallel processing, and async RPC patterns in MaDRPC.

---

## Creating Your Own Examples

To create your own example:

1. **Create a new JavaScript file** in this directory:
   ```bash
   touch examples/my-example.js
   ```

2. **Register your functions** using the `madrpc.register()` API:
   ```javascript
   'use strict';

   madrpc.register('my_function', (args) => {
       const { param1, param2 } = args;
       // Your logic here
       return { result: /* your result */ };
   });
   ```

3. **For distributed computation**, use `madrpc.call()` with async/await:
   ```javascript
   madrpc.register('distributed_function', async (args) => {
       const promises = [];
       for (let i = 0; i < args.numNodes; i++) {
           promises.push(madrpc.call('other_function', { arg: i }));
       }
       const results = await Promise.all(promises);
       return { aggregated: /* aggregate results */ };
   });
   ```

4. **Run your example**:
   ```bash
   cargo run --bin madrpc -- node -s examples/my-example.js -b 0.0.0.0:9001
   cargo run --bin madrpc -- call 127.0.0.1:8080 my_function '{"param1": "value1"}'
   ```

## Key Concepts

### Function Registration

Use `madrpc.register(name, handler)` to register RPC functions:

```javascript
madrpc.register('function_name', (args) => {
    // args is a JSON object with the request parameters
    return { /* return value as a JSON object */ };
});
```

### Async Functions

For async operations (including distributed RPC), use async functions:

```javascript
madrpc.register('async_function', async (args) => {
    const result = await someAsyncOperation();
    return { result };
});
```

### Distributed RPC

Use `madrpc.call()` to make RPC calls to other nodes:

```javascript
madrpc.register('orchestrator', async (args) => {
    const result1 = await madrpc.call('function1', { arg: 1 });
    const result2 = await madrpc.call('function2', { arg: 2 });
    return { combined: /* combine results */ };
});
```

### Parallel Execution

Use `Promise.all()` for parallel execution:

```javascript
const promises = [];
for (let i = 0; i < 10; i++) {
    promises.push(madrpc.call('function', { id: i }));
}
const results = await Promise.all(promises);
```

### Error Handling

Throw errors to return error responses:

```javascript
madrpc.register('safe_divide', (args) => {
    if (args.b === 0) {
        throw new Error('Division by zero is not allowed');
    }
    return { result: args.a / args.b };
});
```

## Tips for Writing Examples

1. **Keep it simple**: Focus on demonstrating one or two concepts per example
2. **Add comments**: Explain what each part of the code does
3. **Use descriptive names**: Make function names and variable names clear
4. **Handle errors**: Include error handling where appropriate
5. **Provide examples**: Show example calls in comments or documentation
6. **Test locally**: Verify your examples work before sharing

## Additional Resources

- [Main README](../README.md) - Project overview and architecture
- [CLAUDE.md](../CLAUDE.md) - Development guidelines and codebase documentation
- [crates/madrpc-common](../crates/madrpc-common/) - Protocol and transport layer
- [crates/madrpc-server](../crates/madrpc-server/) - Node implementation
- [crates/madrpc-client](../crates/madrpc-client/) - RPC client implementation
