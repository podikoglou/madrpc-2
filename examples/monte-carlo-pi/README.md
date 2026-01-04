# Monte Carlo Pi Example

This example demonstrates distributed computation with MaDRPC by estimating Pi using Monte Carlo simulation.

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
Computing Pi using Monte Carlo with 10000000 samples...
Using 3 nodes, 3333333 samples per node
Node 0: 2617682 inside out of 3333333 samples
Node 1: 2617921 inside out of 3333333 samples
Node 2: 2617523 inside out of 3333333 samples

=== Results ===
Total inside: 7853126
Total samples: 10000000
Pi estimate:  3.1412504000
Actual Pi:     3.1415926535
Error:         0.0003422535
```

## How it works

1. Each node runs the same JavaScript file
2. The JavaScript registers a `monte_carlo_sample` RPC function
3. The client calls this RPC on multiple nodes in parallel
4. Each node uses a different random seed (via LCG)
5. Results are aggregated to compute Pi
