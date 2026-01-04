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
