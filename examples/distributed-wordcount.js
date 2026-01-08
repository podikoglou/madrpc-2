'use strict';

// Map-Reduce word count: distribute work across nodes, then aggregate results

// MAP: Count words in a text chunk
madrpc.register('map_wordcount', (args) => {
    const { text, chunkId } = args;

    const words = text.toLowerCase().split(/\W+/);
    const wordCounts = {};

    for (const word of words) {
        if (word.length < 2) continue;
        wordCounts[word] = (wordCounts[word] || 0) + 1;
    }

    return {
        chunkId: chunkId,
        wordCounts: wordCounts,
        uniqueWords: Object.keys(wordCounts).length
    };
});

// MAP with stop word filtering
madrpc.register('map_wordcount_filtered', (args) => {
    const { text, chunkId, minWordLength = 2 } = args;

    const stopWords = new Set([
        'the', 'be', 'to', 'of', 'and', 'a', 'in', 'that', 'have',
        'i', 'it', 'for', 'not', 'on', 'with', 'he', 'as', 'you',
        'do', 'at', 'this', 'but', 'his', 'by', 'from', 'they',
        'we', 'say', 'her', 'she', 'or', 'an', 'will', 'my',
        'one', 'all', 'would', 'there', 'their', 'what', 'so',
        'up', 'out', 'if', 'about', 'who', 'get', 'which', 'go'
    ]);

    const words = text
        .toLowerCase()
        .split(/\W+/)
        .filter(word => word.length >= minWordLength && !stopWords.has(word));

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

// REDUCE: Aggregate word counts from all chunks
madrpc.register('reduce_wordcount', async (args) => {
    const { chunks, filtered = false, minWordLength = 2 } = args;

    const mapFunction = filtered ? 'map_wordcount_filtered' : 'map_wordcount';

    console.log(`Starting distributed word count...`);
    console.log(`- Chunks to process: ${chunks.length}`);
    console.log(`- Map function: ${mapFunction}`);

    // Distribute chunks across nodes via parallel RPC calls
    const mapPromises = chunks.map((chunk, index) =>
        madrpc.call(mapFunction, {
            text: chunk,
            chunkId: index,
            minWordLength: minWordLength
        })
    );

    const mapResults = await Promise.all(mapPromises);
    console.log(`Map phase complete: ${mapResults.length} chunks processed`);

    // Merge all word counts
    const finalCounts = {};
    let totalUniqueWords = 0;
    let totalWordsProcessed = 0;

    for (const result of mapResults) {
        for (const [word, count] of Object.entries(result.wordCounts)) {
            finalCounts[word] = (finalCounts[word] || 0) + count;
        }

        totalUniqueWords += result.uniqueWords;
        if (result.totalWords !== undefined) {
            totalWordsProcessed += result.totalWords;
        }
    }

    // Get top 10 words
    const sortedWords = Object.entries(finalCounts)
        .sort((a, b) => b[1] - a[1])
        .slice(0, 10);

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

// Benchmark parallel execution
madrpc.register('benchmark_wordcount', async (args) => {
    const { chunks, iterations = 3 } = args;

    console.log(`\n=== Benchmarking Word Count ===`);
    console.log(`Chunks: ${chunks.length}`);
    console.log(`Iterations: ${iterations}\n`);

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

// Generate sample data for testing
madrpc.register('generate_sample_data', (args) => {
    const { numChunks = 10, wordsPerChunk = 1000, vocabulary = null } = args;

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

// End-to-end workflow: generate data, map, reduce, display results
madrpc.register('run_wordcount', async (args) => {
    const { numChunks = 20, wordsPerChunk = 500, filtered = true } = args;

    console.log('\n╔════════════════════════════════════════════════════════════╗');
    console.log('║  Distributed Map-Reduce Word Count                        ║');
    console.log('╚════════════════════════════════════════════════════════════╝\n');

    console.log('Step 1: Generating sample data...');
    const dataResult = await madrpc.call('generate_sample_data', {
        numChunks: numChunks,
        wordsPerChunk: wordsPerChunk
    });

    console.log('\nStep 2: Running map-reduce...');
    const reduceResult = await madrpc.call('reduce_wordcount', {
        chunks: dataResult.chunks,
        filtered: filtered
    });

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
