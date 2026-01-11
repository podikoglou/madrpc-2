// Criterion benchmarks for madrpc-orchestrator
//
// Run benchmarks with:
//   cargo bench -p madrpc-orchestrator
//
// For detailed output with plots:
//   cargo bench -p madrpc-orchestrator -- --save-baseline main

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use madrpc_orchestrator::LoadBalancer;
use madrpc_orchestrator::node::{CircuitBreakerConfig, HealthCheckStatus};

fn bench_load_balancer_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("load_balancer_creation");

    group.bench_function("new_2_nodes", |b| {
        let nodes = vec!["node1".to_string(), "node2".to_string()];
        b.iter(|| LoadBalancer::new(black_box(nodes.clone())));
    });

    group.bench_function("new_10_nodes", |b| {
        let nodes: Vec<String> = (0..10).map(|i| format!("node{}", i)).collect();
        b.iter(|| LoadBalancer::new(black_box(nodes.clone())));
    });

    group.bench_function("new_50_nodes", |b| {
        let nodes: Vec<String> = (0..50).map(|i| format!("node{}", i)).collect();
        b.iter(|| LoadBalancer::new(black_box(nodes.clone())));
    });

    group.bench_function("with_config", |b| {
        let nodes = vec!["node1".to_string(), "node2".to_string()];
        let config = CircuitBreakerConfig::default();
        b.iter(|| LoadBalancer::with_config(black_box(nodes.clone()), black_box(config.clone())));
    });

    group.finish();
}

fn bench_next_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("next_node");

    for node_count in [2, 5, 10, 20].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(node_count), node_count, |b, &count| {
            let nodes: Vec<String> = (0..count).map(|i| format!("node{}", i)).collect();
            b.iter(|| {
                let mut lb = LoadBalancer::new(nodes.clone());
                black_box(&mut lb).next_node();
            });
        });
    }

    group.finish();
}

fn bench_round_robin_distribution(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_robin_distribution");

    group.bench_function("10_nodes_100_calls", |b| {
        let nodes: Vec<String> = (0..10).map(|i| format!("node{}", i)).collect();
        b.iter(|| {
            let mut lb = LoadBalancer::new(nodes.clone());
            for _ in 0..100 {
                black_box(&mut lb).next_node();
            }
        });
    });

    group.bench_function("20_nodes_200_calls", |b| {
        let nodes: Vec<String> = (0..20).map(|i| format!("node{}", i)).collect();
        b.iter(|| {
            let mut lb = LoadBalancer::new(nodes.clone());
            for _ in 0..200 {
                black_box(&mut lb).next_node();
            }
        });
    });

    group.finish();
}

fn bench_enable_disable_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("enable_disable_node");

    group.bench_function("disable_node", |b| {
        let nodes = vec!["node1".to_string(), "node2".to_string()];
        b.iter(|| {
            let mut lb = LoadBalancer::new(nodes.clone());
            lb.disable_node("node1");
        });
    });

    group.bench_function("enable_node", |b| {
        let nodes = vec!["node1".to_string(), "node2".to_string()];
        b.iter(|| {
            let mut lb = LoadBalancer::new(nodes.clone());
            lb.disable_node("node1");
            lb.enable_node("node1");
        });
    });

    group.finish();
}

fn bench_add_remove_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("add_remove_node");

    group.bench_function("add_node", |b| {
        let nodes = vec!["node1".to_string()];
        b.iter(|| {
            let mut lb = LoadBalancer::new(nodes.clone());
            lb.add_node("new_node".to_string());
        });
    });

    group.bench_function("remove_node", |b| {
        let nodes = vec!["node1".to_string(), "node2".to_string()];
        b.iter(|| {
            let mut lb = LoadBalancer::new(nodes.clone());
            lb.remove_node("node2");
        });
    });

    group.finish();
}

fn bench_update_health_status(c: &mut Criterion) {
    let mut group = c.benchmark_group("update_health_status");

    group.bench_function("healthy", |b| {
        let nodes = vec!["node1".to_string()];
        let mut lb = LoadBalancer::new(nodes);
        b.iter(|| {
            lb.update_health_status(black_box("node1"), black_box(HealthCheckStatus::Healthy));
        });
    });

    group.bench_function("unhealthy", |b| {
        let nodes = vec!["node1".to_string()];
        let mut lb = LoadBalancer::new(nodes);
        b.iter(|| {
            lb.update_health_status(
                black_box("node1"),
                black_box(HealthCheckStatus::Unhealthy("error".to_string())),
            );
        });
    });

    group.finish();
}

fn bench_get_all_nodes(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_all_nodes");

    for node_count in [2, 10, 50].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(node_count), node_count, |b, &count| {
            let nodes: Vec<String> = (0..count).map(|i| format!("node{}", i)).collect();
            let lb = LoadBalancer::new(nodes);
            b.iter(|| lb.all_nodes());
        });
    }

    group.finish();
}

fn bench_get_enabled_nodes(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_enabled_nodes");

    let nodes: Vec<String> = (0..10).map(|i| format!("node{}", i)).collect();
    let mut lb = LoadBalancer::new(nodes);
    lb.disable_node("node2");
    lb.disable_node("node5");

    group.bench_function("10_nodes_8_enabled", |b| {
        b.iter(|| lb.enabled_nodes());
    });

    group.finish();
}

fn bench_get_disabled_nodes(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_disabled_nodes");

    let nodes: Vec<String> = (0..10).map(|i| format!("node{}", i)).collect();
    let mut lb = LoadBalancer::new(nodes);
    lb.disable_node("node2");
    lb.disable_node("node5");

    group.bench_function("10_nodes_2_disabled", |b| {
        b.iter(|| lb.disabled_nodes());
    });

    group.finish();
}

fn bench_circuit_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_state");

    let nodes = vec!["node1".to_string()];
    let lb = LoadBalancer::new(nodes);

    group.bench_function("get_circuit_state", |b| {
        b.iter(|| lb.circuit_state(black_box("node1")));
    });

    group.finish();
}

fn bench_concurrent_next_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_next_node");

    group.bench_function("concurrent_access", |b| {
        let nodes = vec!["node1".to_string(), "node2".to_string(), "node3".to_string()];
        let lb = std::sync::Arc::new(std::sync::Mutex::new(LoadBalancer::new(nodes)));

        b.iter(|| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let lb_clone = std::sync::Arc::clone(&lb);
                    std::thread::spawn(move || {
                        for _ in 0..10 {
                            let mut lb = lb_clone.lock().unwrap();
                            black_box(&mut *lb).next_node();
                        }
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap();
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_load_balancer_creation,
    bench_next_node,
    bench_round_robin_distribution,
    bench_enable_disable_node,
    bench_add_remove_node,
    bench_update_health_status,
    bench_get_all_nodes,
    bench_get_enabled_nodes,
    bench_get_disabled_nodes,
    bench_circuit_state,
    bench_concurrent_next_node,
);
criterion_main!(benches);
