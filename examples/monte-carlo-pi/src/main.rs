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

    println!("Computing Pi using Monte Carlo...");
    println!("JavaScript will orchestrate {} parallel calls", num_nodes);
    println!("Total samples: {}", total_samples);

    // Single RPC call to JavaScript aggregate function
    // JavaScript handles all parallelization using madrpc.call() and Promise.all()
    let args = json!({
        "numNodes": num_nodes,
        "samplesPerNode": samples_per_node
    });

    let result = client.call("aggregate", args).await?;

    // Extract and display results from the JS-aggregated response
    let total_inside: u64 = serde_json::from_value(result["totalInside"].clone())?;
    let total_samples_result: u64 = serde_json::from_value(result["totalSamples"].clone())?;
    let pi_estimate: f64 = serde_json::from_value(result["piEstimate"].clone())?;

    println!("\n=== Results ===");
    println!("Total inside: {}", total_inside);
    println!("Total samples: {}", total_samples_result);
    println!("Pi estimate:  {:.10}", pi_estimate);
    println!("Actual Pi:     {:.10}", std::f64::consts::PI);
    println!(
        "Error:         {:.10}",
        (pi_estimate - std::f64::consts::PI).abs()
    );

    Ok(())
}
