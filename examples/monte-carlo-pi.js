'use strict';

// Monte Carlo Pi estimation using distributed RPC
// Algorithm: Pi ≈ 4 * (points inside circle) / (total points)

// Linear Congruential Generator for reproducible random numbers
class LCG {
    constructor(seed) {
        this.state = seed;
    }

    next() {
        this.state = (this.state * 1103515245 + 12345) & 0x7fffffff;
        return this.state / 0x7fffffff;
    }
}

// Perform Monte Carlo sampling on a single node
madrpc.register('monte_carlo_sample', (args) => {
    const samples = args.samples || 1000000;
    const seed = args.seed || 0;

    const rng = new LCG(seed);
    let inside = 0;

    for (let i = 0; i < samples; i++) {
        const x = rng.next();
        const y = rng.next();
        if (x * x + y * y <= 1) {
            inside++;
        }
    }

    return {
        inside: inside,
        total: samples
    };
});

// Orchestrate parallel sampling across multiple nodes
madrpc.register('aggregate', async (args) => {
    const numNodes = args.numNodes || 50;
    const samplesPerNode = args.samplesPerNode || 200000;

    // Distribute work across nodes via parallel RPC calls
    const promises = [];
    for (let i = 0; i < numNodes; i++) {
        promises.push(madrpc.call('monte_carlo_sample', {
            samples: samplesPerNode,
            seed: i
        }));
    }

    const results = await Promise.all(promises);

    // Aggregate results
    let totalInside = 0;
    let totalSamples = 0;

    for (const result of results) {
        totalInside += result.inside;
        totalSamples += result.total;
    }

    return {
        totalInside: totalInside,
        totalSamples: totalSamples,
        piEstimate: 4 * totalInside / totalSamples
    };
});

// Single-node estimation for performance comparison
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
