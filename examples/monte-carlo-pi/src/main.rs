use anyhow::Result;
use madrpc_client::MadrpcClient;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    tracing_subscriber::fmt::init();

    // Connect to orchestrator
    let client = MadrpcClient::new("127.0.0.1:8080").await?;

    // Total samples to compute
    let total_samples = 10_000_000;
    let num_nodes = 50;
    let samples_per_node = total_samples / num_nodes;

    println!(
        "Computing Pi using Monte Carlo with {} samples...",
        total_samples
    );
    println!(
        "Using {} nodes, {} samples per node",
        num_nodes, samples_per_node
    );

    // Spawn parallel requests to multiple nodes
    let mut tasks = Vec::new();

    for i in 0..num_nodes {
        let client = client.clone();
        let task = tokio::spawn(async move {
            let args = json!({
                "samples": samples_per_node,
                "seed": i, // Different seed for each node
            });

            client.call("monte_carlo_sample", args).await
        });
        tasks.push(task);
    }

    // Wait for all results and aggregate
    let mut total_inside = 0u64;

    for (i, task) in tasks.into_iter().enumerate() {
        match task.await {
            Ok(Ok(result)) => {
                let inside: u64 = serde_json::from_value(result["inside"].clone()).unwrap_or(0);
                let total: u64 = serde_json::from_value(result["total"].clone()).unwrap_or(0);
                println!("Node {}: {} inside out of {} samples", i, inside, total);
                total_inside += inside;
            }
            Ok(Err(e)) => {
                eprintln!("Node {} failed: {}", i, e);
            }
            Err(e) => {
                eprintln!("Node {} task failed: {}", i, e);
            }
        }
    }

    // Calculate Pi
    let pi_estimate = 4.0 * (total_inside as f64) / (total_samples as f64);

    println!("\n=== Results ===");
    println!("Total inside: {}", total_inside);
    println!("Total samples: {}", total_samples);
    println!("Pi estimate:  {:.10}", pi_estimate);
    println!("Actual Pi:     {:.10}", std::f64::consts::PI);
    println!(
        "Error:         {:.10}",
        (pi_estimate - std::f64::consts::PI).abs()
    );

    Ok(())
}
