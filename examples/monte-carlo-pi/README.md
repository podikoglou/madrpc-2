# Monte Carlo Pi Example

This example demonstrates **JavaScript-driven distributed computation** with MaDRPC by estimating Pi using Monte Carlo simulation with async RPC calls.

## Architecture

This example showcases MaDRPC's new async RPC capability, where JavaScript code orchestrates parallel computation:

- **`monte_carlo_sample`**: Basic Monte Carlo sampling function (generates random points, counts those inside unit circle)
- **`aggregate`**: Async function that uses `madrpc.call()` and `Promise.all()` to parallelize across multiple nodes

The key innovation is that **JavaScript handles the parallel orchestration**, not Rust. The Rust client makes a single RPC call to `aggregate`, which then fans out to multiple nodes in parallel using async/await.

## How it works

The Monte Carlo method for Pi estimation works by generating random points in a unit square and counting how many fall inside the unit circle. The ratio of points inside the circle to total points approaches Pi/4.

## Running the example

### 1. Start the orchestrator
```bash
cargo run --bin madrpc -- orchestrator --node localhost:9001 --node localhost:9002 --node localhost:9003
```

### 2. Start 3 nodes (in separate terminals)
```bash
# Terminal 2
cargo run --bin madrpc -- node --script examples/monte-carlo-pi/scripts/pi.js --bind 0.0.0.0:9001

# Terminal 3
cargo run --bin madrpc -- node --script examples/monte-carlo-pi/scripts/pi.js --bind 0.0.0.0:9002

# Terminal 4
cargo run --bin madrpc -- node --script examples/monte-carlo-pi/scripts/pi.js --bind 0.0.0.0:9003
```

### 3. Run the computation
```bash
cargo run -p monte-carlo-pi
```

## Expected output

```
Computing Pi using Monte Carlo...
JavaScript will orchestrate 50 parallel calls
Total samples: 10000000

=== Results ===
Total inside: 7853126
Total samples: 10000000
Pi estimate:  3.1412504000
Actual Pi:     3.1415926535
Error:         0.0003422535
```

## JavaScript-driven parallelization

The key difference from traditional RPC systems is that **JavaScript orchestrates the parallel work**:

1. **Rust client** makes a single RPC call to `aggregate`
2. **JavaScript `aggregate` function**:
   - Creates an array of promises using `madrpc.call('monte_carlo_sample', ...)`
   - Uses `Promise.all()` to wait for all parallel RPC calls to complete
   - Aggregates results (sums up `inside` counts)
   - Returns the final Pi estimate
3. **Multiple nodes** execute the `monte_carlo_sample` function in parallel
4. **Different seeds** ensure each node generates different random samples

This demonstrates MaDRPC's async RPC capability, where JavaScript code can:
- Make async RPC calls using `await madrpc.call(method, args)`
- Parallelize work using `Promise.all()`
- Compose complex distributed algorithms from simple RPC functions

## Code highlights

### JavaScript (pi.js)

```javascript
// Aggregate function orchestrates parallel sampling
madrpc.register('aggregate', async (args) => {
    const numNodes = args.numNodes || 50;
    const samplesPerNode = args.samplesPerNode || 200000;

    // Create promises for parallel RPC calls
    const promises = [];
    for (let i = 0; i < numNodes; i++) {
        promises.push(madrpc.call('monte_carlo_sample', {
            samples: samplesPerNode,
            seed: i
        }));
    }

    // Wait for all calls and aggregate results
    const results = await Promise.all(promises);
    let totalInside = 0;
    for (const result of results) {
        totalInside += result.inside;
    }

    return {
        totalInside: totalInside,
        totalSamples: numNodes * samplesPerNode,
        piEstimate: 4 * totalInside / (numNodes * samplesPerNode)
    };
});
```

### Rust (main.rs)

```rust
// Single RPC call - JavaScript handles the rest
let result = client.call("aggregate", json!({
    "numNodes": 50,
    "samplesPerNode": 200000
})).await?;

let pi_estimate: f64 = serde_json::from_value(result["piEstimate"].clone())?;
```
