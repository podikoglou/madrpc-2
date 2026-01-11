// Criterion benchmarks for madrpc-common protocol layer
//
// Run benchmarks with:
//   cargo bench -p madrpc-common
//
// For detailed output with plots:
//   cargo bench -p madrpc-common -- --save-baseline main

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use madrpc_common::{Request, Response, MadrpcError};
use serde_json::json;

fn bench_request_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_creation");

    group.bench_function("simple_request", |b| {
        b.iter(|| {
            Request::new(
                black_box("test_method"),
                black_box(json!({"arg": 42})),
            )
        });
    });

    group.bench_function("request_with_timeout", |b| {
        b.iter(|| {
            Request::new(
                black_box("test_method"),
                black_box(json!({"arg": 42})),
            )
            .with_timeout(black_box(5000))
        });
    });

    group.bench_function("request_with_idempotency", |b| {
        b.iter(|| {
            Request::new(
                black_box("test_method"),
                black_box(json!({"arg": 42})),
            )
            .with_idempotency_key(black_box("key-123"))
        });
    });

    group.finish();
}

fn bench_request_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_serialization");

    group.bench_function("serialize_small", |b| {
        let req = Request::new("method", json!({"value": 42}));
        b.iter(|| serde_json::to_string(black_box(&req)));
    });

    group.bench_function("serialize_medium", |b| {
        let req = Request::new("method", json!({"values": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]}));
        b.iter(|| serde_json::to_string(black_box(&req)));
    });

    group.bench_function("serialize_large", |b| {
        let data: Vec<String> = (0..100).map(|i| format!("item_{}", i)).collect();
        let req = Request::new("method", json!({ "data": data }));
        b.iter(|| serde_json::to_string(black_box(&req)));
    });

    group.finish();
}

fn bench_request_deserialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_deserialization");

    let small_json = r#"{"id":1,"method":"test","params":{"value":42},"timeout_ms":null,"idempotency_key":null}"#;
    let medium_json = r#"{"id":1,"method":"test","params":{"values":[1,2,3,4,5,6,7,8,9,10]},"timeout_ms":null,"idempotency_key":null}"#;
    let large_json = r#"{"id":1,"method":"test","params":{"data":["item_0","item_1","item_2","item_3","item_4","item_5","item_6","item_7","item_8","item_9","item_10","item_11","item_12","item_13","item_14","item_15","item_16","item_17","item_18","item_19"]},"timeout_ms":null,"idempotency_key":null}"#;

    group.bench_function("deserialize_small", |b| {
        b.iter(|| serde_json::from_str::<Request>(black_box(small_json)));
    });

    group.bench_function("deserialize_medium", |b| {
        b.iter(|| serde_json::from_str::<Request>(black_box(medium_json)));
    });

    group.bench_function("deserialize_large", |b| {
        b.iter(|| serde_json::from_str::<Request>(black_box(large_json)));
    });

    group.finish();
}

fn bench_response_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("response_creation");

    group.bench_function("success_response", |b| {
        b.iter(|| Response::success(black_box(1), black_box(json!({"result": 42}))));
    });

    group.bench_function("error_response", |b| {
        b.iter(|| {
            Response::error(
                black_box(1),
                black_box("connection failed"),
            )
        });
    });

    group.finish();
}

fn bench_request_cloning(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_cloning");

    let small_request = Request::new("method", json!({"v": 1}));
    let medium_request = Request::new("method", json!({"v": [1, 2, 3, 4, 5]}));
    let data: Vec<String> = (0..50).map(|i| format!("key_{}", i)).collect();
    let large_request = Request::new("method", json!({ "data": data }));

    group.bench_function("clone_small", |b| {
        b.iter(|| black_box(&small_request).clone());
    });

    group.bench_function("clone_medium", |b| {
        b.iter(|| black_box(&medium_request).clone());
    });

    group.bench_function("clone_large", |b| {
        b.iter(|| black_box(&large_request).clone());
    });

    group.finish();
}

fn bench_response_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("response_serialization");

    let success_response = Response::success(1, json!({"result": 42}));
    let error_response = Response::error(1, "connection failed");

    group.bench_function("serialize_success", |b| {
        b.iter(|| serde_json::to_string(black_box(&success_response)));
    });

    group.bench_function("serialize_error", |b| {
        b.iter(|| serde_json::to_string(black_box(&error_response)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_request_creation,
    bench_request_serialization,
    bench_request_deserialization,
    bench_response_creation,
    bench_request_cloning,
    bench_response_serialization,
);
criterion_main!(benches);
