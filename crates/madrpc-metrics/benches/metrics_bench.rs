// Criterion benchmarks for madrpc-metrics
//
// Run benchmarks with:
//   cargo bench -p madrpc-metrics
//
// For detailed output with plots:
//   cargo bench -p madrpc-metrics -- --save-baseline main

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use madrpc_metrics::{NodeMetricsCollector, OrchestratorMetricsCollector, MetricsCollector, MetricsConfig};
use std::time::Instant;

fn bench_record_call(c: &mut Criterion) {
    let mut group = c.benchmark_group("record_call");

    let collector = NodeMetricsCollector::new();

    group.bench_function("single_call", |b| {
        b.iter(|| {
            collector.record_call(black_box("test_method"), black_box(Instant::now()), black_box(true));
        });
    });

    group.finish();
}

fn bench_record_many_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("record_many_calls");

    let collector = NodeMetricsCollector::new();

    // Benchmark recording calls to multiple methods
    let methods = vec!["method_a", "method_b", "method_c", "method_d", "method_e"];

    group.bench_function("five_methods", |b| {
        b.iter(|| {
            for method in &methods {
                collector.record_call(black_box(method), black_box(Instant::now()), black_box(true));
            }
        });
    });

    group.finish();
}

fn bench_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot");

    let collector = NodeMetricsCollector::new();

    // Record some calls first
    for i in 0..100 {
        let method = format!("method_{}", i % 10);
        collector.record_call(&method, Instant::now(), i % 2 == 0);
    }

    group.bench_function("snapshot_10_methods", |b| {
        b.iter(|| collector.snapshot());
    });

    group.finish();
}

fn bench_record_node_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("record_node_request");

    let collector = OrchestratorMetricsCollector::new();

    group.bench_function("single_node", |b| {
        b.iter(|| {
            collector.record_node_request(black_box("127.0.0.1:9001"));
        });
    });

    group.bench_function("multiple_nodes", |b| {
        let nodes = vec![
            "127.0.0.1:9001",
            "127.0.0.1:9002",
            "127.0.0.1:9003",
            "127.0.0.1:9004",
            "127.0.0.1:9005",
        ];
        let mut idx = 0;
        b.iter(|| {
            collector.record_node_request(black_box(nodes[idx % nodes.len()]));
            idx += 1;
        });
    });

    group.finish();
}

fn bench_is_metrics_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("is_metrics_request");

    let collector = NodeMetricsCollector::new();

    group.bench_function("builtin_method", |b| {
        b.iter(|| collector.is_metrics_request(black_box("_metrics")));
    });

    group.bench_function("custom_method", |b| {
        b.iter(|| collector.is_metrics_request(black_box("custom_method")));
    });

    group.finish();
}

fn bench_handle_metrics_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle_metrics_request");

    let collector = NodeMetricsCollector::new();

    // Record some calls first
    for i in 0..100 {
        let method = format!("method_{}", i % 10);
        collector.record_call(&method, Instant::now(), i % 2 == 0);
    }

    group.bench_function("handle_metrics", |b| {
        b.iter(|| collector.handle_metrics_request(black_box("_metrics"), black_box(1)));
    });

    group.bench_function("handle_info", |b| {
        b.iter(|| collector.handle_metrics_request(black_box("_info"), black_box(1)));
    });

    group.finish();
}

fn bench_concurrent_recording(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_recording");

    // Test with different numbers of methods
    for method_count in [10, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(method_count), method_count, |b, &count| {
            let collector = std::sync::Arc::new(NodeMetricsCollector::new());
            let methods: Vec<String> = (0..count).map(|i| format!("method_{}", i)).collect();

            b.iter(|| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let handles: Vec<_> = methods
                    .iter()
                    .map(|method| {
                        let collector = collector.clone();
                        let method = method.clone();
                        rt.spawn(async move {
                            for _ in 0..10 {
                                collector.record_call(&method, Instant::now(), true);
                            }
                        })
                    })
                    .collect();

                rt.block_on(async {
                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_record_call,
    bench_record_many_calls,
    bench_snapshot,
    bench_record_node_request,
    bench_is_metrics_request,
    bench_handle_metrics_request,
    bench_concurrent_recording,
);
criterion_main!(benches);
