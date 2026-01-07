'use strict';

// ============================================================================
// Monte Carlo Pi Estimation Example
// ============================================================================
// This example demonstrates distributed parallel computation using madrpc.
//
// The Monte Carlo method estimates Pi by generating random points in a unit
// square and counting how many fall inside the unit circle. The ratio of
// points inside the circle to total points approximates Pi/4.
//
// This example showcases:
// - Distributed computation across multiple nodes
// - Async RPC calls using madrpc.call()
// - Parallel execution with Promise.all()
// - Result aggregation from multiple sources
//
// Algorithm: Pi ≈ 4 * (points inside circle) / (total points)

// ============================================================================
// Linear Congruential Generator (LCG)
// ============================================================================

// A simple pseudo-random number generator for reproducible results.
// LCG is used instead of Math.random() to ensure consistent results
// across different nodes and runs.
class LCG {
    constructor(seed) {
        this.state = seed;
    }

    // Returns a float between 0 and 1
    next() {
        // LCG parameters from GCC's implementation
        this.state = (this.state * 1103515245 + 12345) & 0x7fffffff;
        return this.state / 0x7fffffff;
    }
}

// ============================================================================
// Monte Carlo Sampling Function
// ============================================================================

// Performs Monte Carlo sampling to estimate Pi.
// Each call generates random points and counts how many fall inside
// the unit circle (x² + y² ≤ 1).
//
// Args:
//   samples: Number of random points to generate (default: 1000000)
//   seed: Seed for the random number generator (default: 0)
//
// Returns:
//   inside: Number of points inside the unit circle
//   total: Total number of points generated
madrpc.register('monte_carlo_sample', (args) => {
    const samples = args.samples || 1000000;
    const seed = args.seed || 0;

    const rng = new LCG(seed);
    let inside = 0;

    // Generate random points and count those inside unit circle
    for (let i = 0; i < samples; i++) {
        const x = rng.next();
        const y = rng.next();

        // Check if point is inside unit circle: x² + y² ≤ 1
        if (x * x + y * y <= 1) {
            inside++;
        }
    }

    return {
        inside: inside,
        total: samples
    };
});

// ============================================================================
// Distributed Aggregation Function
// ============================================================================

// Orchestrates parallel Monte Carlo sampling across multiple nodes.
// This function demonstrates how to use madrpc.call() for distributed RPC
// and Promise.all() for parallel execution.
//
// Args:
//   numNodes: Number of parallel RPC calls to make (default: 50)
//   samplesPerNode: Number of samples per RPC call (default: 200000)
//
// Returns:
//   totalInside: Total points inside circle from all nodes
//   totalSamples: Total points sampled from all nodes
//   piEstimate: Final estimate of Pi (4 * inside/total)
//
// Example usage:
//   madrpc.call('aggregate', {
//     numNodes: 50,
//     samplesPerNode: 200000
//   })
madrpc.register('aggregate', async (args) => {
    const numNodes = args.numNodes || 50;
    const samplesPerNode = args.samplesPerNode || 200000;

    // Create an array of promises for parallel RPC calls
    // Each call performs independent Monte Carlo sampling
    const promises = [];
    for (let i = 0; i < numNodes; i++) {
        promises.push(madrpc.call('monte_carlo_sample', {
            samples: samplesPerNode,
            seed: i  // Different seed for each node
        }));
    }

    // Wait for all RPC calls to complete in parallel
    // This is where the distributed computation happens
    const results = await Promise.all(promises);

    // Aggregate results from all nodes
    let totalInside = 0;
    let totalSamples = 0;

    for (const result of results) {
        totalInside += result.inside;
        totalSamples += result.total;
    }

    // Calculate final Pi estimate
    // Pi ≈ 4 * (points inside circle) / (total points)
    const piEstimate = 4 * totalInside / totalSamples;

    return {
        totalInside: totalInside,
        totalSamples: totalSamples,
        piEstimate: piEstimate
    };
});

// ============================================================================
// Single-Node Estimation (for comparison)
// ============================================================================

// Performs Monte Carlo sampling on a single node without distributed calls.
// Useful for comparing single-node vs distributed performance.
//
// Args:
//   totalSamples: Total number of samples to generate (default: 10000000)
//
// Returns:
//   totalInside: Number of points inside the unit circle
//   totalSamples: Total number of points generated
//   piEstimate: Final estimate of Pi
madrpc.register('estimate_single', (args) => {
    const totalSamples = args.totalSamples || 10000000;
    const seed = args.seed || 0;

    const rng = new LCG(seed);
    let inside = 0;

    for (let i = 0; i < totalSamples; i++) {
        const x = rng.next();
        const y = rng.next();

        if (x * x + y * y <= 1) {
            inside++;
        }
    }

    return {
        totalInside: inside,
        totalSamples: totalSamples,
        piEstimate: 4 * inside / totalSamples
    };
});
