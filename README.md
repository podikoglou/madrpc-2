# MaDRPC

Distributed RPC system in Rust. Run JavaScript functions across multiple nodes using Boa.

## Protocol

```
[4-byte length (big-endian)][JSON payload]
```

Request:
```json
{
  "method": "function_name",
  "args": {"key": "value"},
  "id": 123
}
```

Response:
```json
{
  "result": {"key": "value"},
  "error": null,
  "id": 123
}
```

Connections stay alive for multiple requests.

## CLI

```bash
# Orchestrator (load balancer)
madrpc orchestrator -b 0.0.0.0:8080 -n 127.0.0.1:9001 -n 127.0.0.1:9002

# Node (compute worker)
madrpc node -s script.js -b 0.0.0.0:9001 --pool-size 8

# Monitor
madrpc top 127.0.0.1:8080
```

## Writing RPCs

Create a JavaScript file and load it with the node:

```javascript
// Register a function
madrpc.register('add', (args) => {
    const a = args.a || 0;
    const b = args.b || 0;
    return { sum: a + b };
});
```

### Calling other RPCs from within an RPC

```javascript
madrpc.register('parallel_sum', async (args) => {
    const numbers = args.numbers || [];

    // Call another RPC
    const result1 = await madrpc.call('add', { a: numbers[0], b: numbers[1] });
    const result2 = await madrpc.call('add', { a: numbers[2], b: numbers[3] });

    return { total: result1.sum + result2.sum };
});
```

### Async RPCs

```javascript
madrpc.register('fetch_data', async (args) => {
    // Do async work
    const response = await fetch(args.url);
    const data = await response.json();
    return { data };
});
```

## Examples

```bash
# Run the Monte Carlo Pi example
cargo run -p monte-carlo-pi
```

Example script (`examples/monte-carlo-pi/scripts/pi.js`):

```javascript
// LCG random number generator
class LCG {
    constructor(seed) { this.state = seed; }
    next() {
        this.state = (this.state * 1103515245 + 12345) & 0x7fffffff;
        return this.state / 0x7fffffff;
    }
}

// Monte Carlo sampling
madrpc.register('monte_carlo_sample', (args) => {
    const samples = args.samples || 1000000;
    const seed = args.seed || 0;
    const rng = new LCG(seed);
    let inside = 0;

    for (let i = 0; i < samples; i++) {
        const x = rng.next();
        const y = rng.next();
        if (x * x + y * y <= 1) inside++;
    }

    return { inside, total: samples };
});
```
