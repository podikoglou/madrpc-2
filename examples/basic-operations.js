'use strict';

// Basic RPC operations demonstrating the madrpc.register() API

// Number operations
madrpc.register('add', (args) => {
    const { a, b } = args;
    return { result: a + b };
});

madrpc.register('subtract', (args) => {
    const { a, b } = args;
    return { result: a - b };
});

madrpc.register('multiply', (args) => {
    const { a, b } = args;
    return { result: a * b };
});

madrpc.register('divide', (args) => {
    const { a, b } = args;

    if (b === 0) {
        throw new Error('Division by zero is not allowed');
    }

    return {
        result: a / b,
        quotient: Math.floor(a / b),
        remainder: a % b
    };
});

// String operations
madrpc.register('concatenate', (args) => {
    const { str1, str2, separator = '' } = args;
    return { result: str1 + separator + str2 };
});

madrpc.register('reverse', (args) => {
    const { str } = args;
    return { result: str.split('').reverse().join('') };
});

madrpc.register('uppercase', (args) => {
    const { str } = args;
    return { result: str.toUpperCase() };
});

madrpc.register('lowercase', (args) => {
    const { str } = args;
    return { result: str.toLowerCase() };
});

// JSON operations
madrpc.register('merge', (args) => {
    const { obj1, obj2 } = args;
    return {
        result: {
            ...obj1,
            ...obj2
        }
    };
});

madrpc.register('deep_clone', (args) => {
    const { obj } = args;
    return {
        result: JSON.parse(JSON.stringify(obj))
    };
});

madrpc.register('pick', (args) => {
    const { obj, fields } = args;
    const result = {};

    for (const field of fields) {
        if (field in obj) {
            result[field] = obj[field];
        }
    }

    return { result };
});

// Aggregation example: combine multiple operations
madrpc.register('math_pipeline', (args) => {
    const { initial, operations } = args;
    let result = initial;

    for (const op of operations) {
        switch (op.operation) {
            case 'add':
                result += op.value;
                break;
            case 'subtract':
                result -= op.value;
                break;
            case 'multiply':
                result *= op.value;
                break;
            case 'divide':
                if (op.value === 0) {
                    throw new Error('Division by zero in pipeline');
                }
                result /= op.value;
                break;
            default:
                throw new Error(`Unknown operation: ${op.operation}`);
        }
    }

    return { result };
});

// Count word occurrences, return top 10
madrpc.register('word_count', (args) => {
    const { text } = args;
    const words = text.toLowerCase().split(/\s+/);
    const counts = {};

    for (const word of words) {
        if (word.length > 0) {
            counts[word] = (counts[word] || 0) + 1;
        }
    }

    return {
        wordCount: words.length,
        uniqueWords: Object.keys(counts).length,
        topWords: Object.entries(counts)
            .sort((a, b) => b[1] - a[1])
            .slice(0, 10)
            .map(([word, count]) => ({ word, count }))
    };
});
