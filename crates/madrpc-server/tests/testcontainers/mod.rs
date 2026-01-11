//! Testcontainers utilities for MaDRPC server integration tests.

use std::time::Duration;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    CopyDataSource,
    GenericImage,
    ImageExt,
};

// ============================================================================
// Common Test Scripts
// ============================================================================

/// Test script with echo, compute, and async_func functions.
pub fn server_test_script() -> String {
    r#"
        madrpc.register('echo', function(args) {
            return args;
        });

        madrpc.register('compute', function(args) {
            return { result: args.x * args.y };
        });

        madrpc.register('async_func', async function(args) {
            // Simulate async work with a Promise
            return Promise.resolve({ result: args.value * 2 });
        });
    "#
    .to_string()
}

/// Empty test script.
pub fn empty_script() -> String {
    "// empty".to_string()
}

// ============================================================================
// Node Container
// ============================================================================

/// Wrapper for a running node container.
pub struct NodeContainer {
    #[allow(dead_code)]
    container: testcontainers::ContainerAsync<GenericImage>,
    /// The host port mapped to the container's port 9001.
    pub host_port: u16,
    /// The external URL for connecting to this node.
    pub external_url: String,
}

impl NodeContainer {
    /// Start a new node container with the given script content.
    pub async fn start(script_content: String) -> anyhow::Result<Self> {
        let image = GenericImage::new("madrpc:test", "")
            .with_exposed_port(9001.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Server listening on"))
            .with_copy_to("/app/script.js", CopyDataSource::Data(script_content.into_bytes()))
            .with_cmd(["node", "-s", "/app/script.js", "-b", "0.0.0.0:9001"]);

        let container = image.start().await?;

        let port = container.get_host_port_ipv4(9001).await?;
        let external_url = format!("http://127.0.0.1:{}", port);

        // Wait for the _info endpoint to be ready
        Self::wait_for_ready(&external_url).await?;

        Ok(Self {
            container,
            host_port: port,
            external_url,
        })
    }

    /// Wait for the node to be ready by polling the _info endpoint.
    async fn wait_for_ready(url: &str) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let info_url = format!("{}/_info", url);

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(30);

        while start.elapsed() < timeout {
            match client.get(&info_url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(_) | Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }

        anyhow::bail!("Node did not become ready within timeout at {}", url);
    }

    /// Get the node's URL for client connections.
    pub fn url(&self) -> &str {
        &self.external_url
    }
}
