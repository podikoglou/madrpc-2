'use strict';

// ============================================================================
// Map-Reduce Word Count Example
// ============================================================================
// This example demonstrates the classic Map-Reduce pattern for distributed
// computing, similar to how Apache Spark processes large datasets.
//
// The computation is split into two phases:
// 1. MAP: Each node processes a chunk of text and produces local word counts
// 2. REDUCE: Results from all nodes are aggregated into final counts
//
// This showcases madrpc's key features:
// - Parallel execution via Promise.all()
// - Distributed coordination using madrpc.call()
// - Async orchestration with async/await
// ============================================================================

// ============================================================================
// MAP PHASE: Count words in a text chunk
// ============================================================================
// This function runs on individual nodes in parallel.
// Each node receives a chunk of text and returns a word frequency map.
//
// Args:
//   - text: A string containing a portion of the document
//   - chunkId: Identifier for this chunk (useful for debugging)
//
// Returns:
//   - wordCounts: Object mapping words to their frequency in this chunk
// ============================================================================
madrpc.register('map_wordcount', (args) => {
    const { text, chunkId } = args;

    // Tokenize: convert to lowercase, split on non-word characters
    const words = text.toLowerCase().split(/\W+/);

    // Count word frequencies in this chunk
    const wordCounts = {};
    for (const word of words) {
        // Skip empty strings and very short words
        if (word.length < 2) continue;

        wordCounts[word] = (wordCounts[word] || 0) + 1;
    }

    return {
        chunkId: chunkId,
        wordCounts: wordCounts,
        uniqueWords: Object.keys(wordCounts).length
    };
});

// ============================================================================
// MAP-V2: Count words with additional filtering
// ============================================================================
// Enhanced version that demonstrates more complex map logic:
// - Filters stop words (common words like "the", "and", etc.)
// - Normalizes whitespace
// - Provides statistics
//
// This showcases how map functions can include arbitrary business logic
// while still being embarrassingly parallel.
// ============================================================================
madrpc.register('map_wordcount_filtered', (args) => {
    const { text, chunkId, minWordLength = 2 } = args;

    // Common stop words to filter out
    const stopWords = new Set([
        'the', 'be', 'to', 'of', 'and', 'a', 'in', 'that', 'have',
        'i', 'it', 'for', 'not', 'on', 'with', 'he', 'as', 'you',
        'do', 'at', 'this', 'but', 'his', 'by', 'from', 'they',
        'we', 'say', 'her', 'she', 'or', 'an', 'will', 'my',
        'one', 'all', 'would', 'there', 'their', 'what', 'so',
        'up', 'out', 'if', 'about', 'who', 'get', 'which', 'go'
    ]);

    // Tokenize and filter
    const words = text
        .toLowerCase()
        .split(/\W+/)
        .filter(word => word.length >= minWordLength && !stopWords.has(word));

    // Count frequencies
    const wordCounts = {};
    for (const word of words) {
        wordCounts[word] = (wordCounts[word] || 0) + 1;
    }

    return {
        chunkId: chunkId,
        wordCounts: wordCounts,
        uniqueWords: Object.keys(wordCounts).length,
        totalWords: words.length
    };
});

// ============================================================================
// REDUCE PHASE: Aggregate word counts from all chunks
// ============================================================================
// This orchestrator function demonstrates the power of distributed computing:
// 1. It receives a large dataset split into chunks
// 2. Distributes each chunk to different nodes via madrpc.call()
// 3. All nodes process their chunks in parallel using Promise.all()
// 4. Aggregates results into a single final word count
//
// Args:
//   - chunks: Array of text strings to process
//   - filtered: Whether to use the filtered map function (default: false)
//   - minWordLength: Minimum word length for filtered mode (default: 2)
//
// Returns:
//   - finalCounts: Aggregated word frequencies across all chunks
//   - stats: Statistics about the computation
// ============================================================================
madrpc.register('reduce_wordcount', async (args) => {
    const { chunks, filtered = false, minWordLength = 2 } = args;

    const mapFunction = filtered ? 'map_wordcount_filtered' : 'map_wordcount';

    console.log(`Starting distributed word count...`);
    console.log(`- Chunks to process: ${chunks.length}`);
    console.log(`- Map function: ${mapFunction}`);

    // ========================================================================
    // PARALLEL MAP: Distribute work across all available nodes
    // ========================================================================
    // Create an array of promises, one for each chunk.
    // Each promise represents an RPC call to a node (load-balanced by orchestrator).
    //
    // The orchestrator uses round-robin load balancing, so if you have 50 chunks
    // and 2 nodes, each node will process ~25 chunks in parallel.
    //
    // This is the key to madrpc's scalability: add more nodes to process
    // more chunks concurrently!
    // ========================================================================
    const mapPromises = chunks.map((chunk, index) =>
        madrpc.call(mapFunction, {
            text: chunk,
            chunkId: index,
            minWordLength: minWordLength
        })
    );

    // Wait for all map operations to complete in parallel
    // This is where the speedup happens - all chunks are processed concurrently!
    const mapResults = await Promise.all(mapPromises);

    console.log(`Map phase complete: ${mapResults.length} chunks processed`);

    // ========================================================================
    // REDUCE: Aggregate results from all nodes
    // ========================================================================
    // Merge all word counts into a single object
    // ========================================================================
    const finalCounts = {};
    let totalUniqueWords = 0;
    let totalWordsProcessed = 0;

    for (const result of mapResults) {
        // Merge word counts from this chunk
        for (const [word, count] of Object.entries(result.wordCounts)) {
            finalCounts[word] = (finalCounts[word] || 0) + count;
        }

        // Accumulate statistics
        totalUniqueWords += result.uniqueWords;
        if (result.totalWords !== undefined) {
            totalWordsProcessed += result.totalWords;
        }
    }

    // Sort words by frequency (most common first)
    const sortedWords = Object.entries(finalCounts)
        .sort((a, b) => b[1] - a[1])
        .slice(0, 10); // Top 10 words

    console.log(`Reduce phase complete: ${Object.keys(finalCounts).length} unique words`);

    return {
        finalCounts: finalCounts,
        stats: {
            totalChunks: chunks.length,
            totalUniqueWords: Object.keys(finalCounts).length,
            sumOfChunkUniqueWords: totalUniqueWords,
            totalWordsProcessed: totalWordsProcessed,
            topWords: sortedWords.map(([word, count]) => ({ word, count }))
        }
    };
});

// ============================================================================
// BENCHMARK: Parallel vs Sequential Comparison
// ============================================================================
// This function demonstrates the performance benefit of parallel execution
// by comparing sequential vs parallel processing of the same data.
//
// Args:
//   - chunks: Array of text strings to process
//   - iterations: Number of times to run the benchmark (default: 3)
//
// Returns:
//   - results: Comparison of sequential vs parallel execution times
// ============================================================================
madrpc.register('benchmark_wordcount', async (args) => {
    const { chunks, iterations = 3 } = args;

    console.log(`\n=== Benchmarking Word Count ===`);
    console.log(`Chunks: ${chunks.length}`);
    console.log(`Iterations: ${iterations}\n`);

    // Benchmark parallel execution (using Promise.all)
    const parallelTimes = [];
    for (let i = 0; i < iterations; i++) {
        const start = Date.now();

        const promises = chunks.map((chunk, idx) =>
            madrpc.call('map_wordcount', { text: chunk, chunkId: idx })
        );
        await Promise.all(promises);

        parallelTimes.push(Date.now() - start);
        console.log(`Parallel run ${i + 1}: ${parallelTimes[i]}ms`);
    }

    const avgParallel = parallelTimes.reduce((a, b) => a + b, 0) / parallelTimes.length;
    console.log(`Average parallel time: ${avgParallel.toFixed(2)}ms\n`);

    return {
        parallel: {
            times: parallelTimes,
            average: avgParallel
        },
        chunks: chunks.length,
        iterations: iterations
    };
});

// ============================================================================
// HELPER: Generate sample text for testing
// ============================================================================
// Creates a synthetic dataset for demonstration purposes.
// In production, you would load real data from files, databases, etc.
//
// Args:
//   - numChunks: Number of text chunks to generate
//   - wordsPerChunk: Approximate words per chunk
//   - vocabulary: Set of words to sample from (optional)
//
// Returns:
//   - chunks: Array of generated text chunks
// ============================================================================
madrpc.register('generate_sample_data', (args) => {
    const { numChunks = 10, wordsPerChunk = 1000, vocabulary = null } = args;

    // Default vocabulary (classic computer science words)
    const defaultVocab = [
        'distributed', 'computing', 'parallel', 'mapreduce', 'spark',
        'hadoop', 'scalability', 'performance', 'algorithm', 'data',
        'processing', 'cluster', 'node', 'network', 'latency',
        'throughput', 'concurrency', 'synchronization', 'sharding',
        'replication', 'consistency', 'availability', 'partition',
        'fault', 'tolerance', 'load', 'balancer', 'orchestrator'
    ];

    const vocab = vocabulary || defaultVocab;

    const chunks = [];
    for (let i = 0; i < numChunks; i++) {
        // Generate random text by sampling vocabulary
        const words = [];
        for (let j = 0; j < wordsPerChunk; j++) {
            const word = vocab[Math.floor(Math.random() * vocab.length)];
            words.push(word);
        }
        chunks.push(words.join(' '));
    }

    console.log(`Generated ${numChunks} chunks with ~${wordsPerChunk} words each`);

    return {
        chunks: chunks,
        totalWords: numChunks * wordsPerChunk,
        vocabularySize: vocab.length
    };
});

// ============================================================================
// FULL WORKFLOW: End-to-end example
// ============================================================================
// Demonstrates the complete workflow: data generation -> map -> reduce
// This is the entry point you would typically call to see the full system.
//
// Args:
//   - numChunks: Number of chunks to generate and process
//   - wordsPerChunk: Words per chunk
//   - filtered: Use filtered word count (default: true)
//
// Returns:
//   - results: Complete workflow results with statistics
// ============================================================================
madrpc.register('run_wordcount', async (args) => {
    const { numChunks = 20, wordsPerChunk = 500, filtered = true } = args;

    console.log('\n╔════════════════════════════════════════════════════════════╗');
    console.log('║  Distributed Map-Reduce Word Count                        ║');
    console.log('╚════════════════════════════════════════════════════════════╝\n');

    // Step 1: Generate sample data (or load from external source)
    console.log('Step 1: Generating sample data...');
    const dataResult = await madrpc.call('generate_sample_data', {
        numChunks: numChunks,
        wordsPerChunk: wordsPerChunk
    });

    // Step 2: Run map-reduce
    console.log('\nStep 2: Running map-reduce...');
    const reduceResult = await madrpc.call('reduce_wordcount', {
        chunks: dataResult.chunks,
        filtered: filtered
    });

    // Step 3: Display results
    console.log('\n╔════════════════════════════════════════════════════════════╗');
    console.log('║  Results                                                   ║');
    console.log('╚════════════════════════════════════════════════════════════╝');
    console.log(`\nTotal chunks processed: ${reduceResult.stats.totalChunks}`);
    console.log(`Total unique words: ${reduceResult.stats.totalUniqueWords}`);
    if (reduceResult.stats.totalWordsProcessed > 0) {
        console.log(`Total words processed: ${reduceResult.stats.totalWordsProcessed}`);
    }
    console.log(`\nTop 10 most common words:`);
    for (const { word, count } of reduceResult.stats.topWords) {
        console.log(`  ${word.padEnd(20)} ${count}`);
    }

    return {
        data: dataResult,
        analysis: reduceResult
    };
});
