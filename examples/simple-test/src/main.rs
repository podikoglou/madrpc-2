use madrpc_client::MadrpcClient;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("Connecting to http://127.0.0.1:18084 (orchestrator)");
    let client = MadrpcClient::new("http://127.0.0.1:18084").await?;

    println!("Calling monte_carlo_sample through orchestrator...");
    match client.call("monte_carlo_sample", json!({"samples": 1000, "seed": 42})).await {
        Ok(result) => {
            println!("Success! Result: {}", result);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    Ok(())
}
