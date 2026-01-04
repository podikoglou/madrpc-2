'use strict';

// Simple test RPC
madrpc.register('echo', (args) => {
    return { message: args.msg, status: 'ok' };
});

madrpc.register('add', (args) => {
    return { sum: args.a + args.b };
});
