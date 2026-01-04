use madrpc_client::MadrpcClient;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("Connecting to 127.0.0.1:8080 (orchestrator)");
    let client = MadrpcClient::new("127.0.0.1:8080").await?;

    println!("Calling echo RPC...");
    match client.call("echo", json!({"msg": "hello from client"})).await {
        Ok(result) => {
            println!("Success! Result: {}", result);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    Ok(())
}
