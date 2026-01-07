'use strict';

// ============================================================================
// Basic RPC Operations Example
// ============================================================================
// This example demonstrates fundamental RPC operations including:
// - Number operations (add, subtract, multiply, divide)
// - String operations (concatenate, reverse, uppercase, lowercase)
// - JSON operations (merge, deep clone, pick fields)
//
// These examples showcase the basic madrpc.register() API and simple
// request/response patterns.

// ============================================================================
// Number Operations
// ============================================================================

// Add two numbers
madrpc.register('add', (args) => {
    const { a, b } = args;
    return { result: a + b };
});

// Subtract two numbers
madrpc.register('subtract', (args) => {
    const { a, b } = args;
    return { result: a - b };
});

// Multiply two numbers
madrpc.register('multiply', (args) => {
    const { a, b } = args;
    return { result: a * b };
});

// Divide two numbers with error handling for division by zero
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

// ============================================================================
// String Operations
// ============================================================================

// Concatenate two strings
madrpc.register('concatenate', (args) => {
    const { str1, str2, separator = '' } = args;
    return { result: str1 + separator + str2 };
});

// Reverse a string
madrpc.register('reverse', (args) => {
    const { str } = args;
    return { result: str.split('').reverse().join('') };
});

// Convert string to uppercase
madrpc.register('uppercase', (args) => {
    const { str } = args;
    return { result: str.toUpperCase() };
});

// Convert string to lowercase
madrpc.register('lowercase', (args) => {
    const { str } = args;
    return { result: str.toLowerCase() };
});

// ============================================================================
// JSON Operations
// ============================================================================

// Merge two JSON objects
madrpc.register('merge', (args) => {
    const { obj1, obj2 } = args;
    return {
        result: {
            ...obj1,
            ...obj2
        }
    };
});

// Deep clone a JSON object
madrpc.register('deep_clone', (args) => {
    const { obj } = args;
    return {
        result: JSON.parse(JSON.stringify(obj))
    };
});

// Pick specific fields from a JSON object
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

// ============================================================================
// Aggregation Example (combining multiple operations)
// ============================================================================

// Apply multiple operations in sequence
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

// Count word occurrences in a text
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
