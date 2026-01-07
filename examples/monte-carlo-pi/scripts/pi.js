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

// Aggregator that orchestrates parallel Monte Carlo sampling
// Uses madrpc.call() for async RPC and Promise.all() for parallelization
madrpc.register('aggregate', async (args) => {
    const numNodes = args.numNodes || 50;
    const samplesPerNode = args.samplesPerNode || 200000;

    // Create array of promises for parallel RPC calls
    const promises = [];
    for (let i = 0; i < numNodes; i++) {
        promises.push(madrpc.call('monte_carlo_sample', {
            samples: samplesPerNode,
            seed: i
        }));
    }

    // Wait for all RPC calls to complete
    const results = await Promise.all(promises);

    // Aggregate results from all nodes
    let totalInside = 0;
    let totalSamples = 0;

    for (const result of results) {
        totalInside += result.inside;
        totalSamples += result.total;
    }

    // Return aggregated results with Pi estimate
    return {
        totalInside: totalInside,
        totalSamples: totalSamples,
        piEstimate: 4 * totalInside / totalSamples
    };
});
