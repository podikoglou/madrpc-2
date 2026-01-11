//! Testcontainers utilities for MaDRPC integration tests.
//!
//! Provides reusable helpers for spinning up madrpc nodes and orchestrators
//! in Docker containers with proper health checks and lifecycle management.

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

/// Common test script with echo, add, compute, and sleep functions.
pub fn common_test_script() -> String {
    r#"
    madrpc.register('echo', (args) => {
        return args;
    });

    madrpc.register('add', (args) => {
        return { result: args.a + args.b };
    });

    madrpc.register('sleep', (args) => {
        const ms = args.ms || 100;
        const start = Date.now();
        while (Date.now() - start < ms) {
            // Busy wait
        }
        return { slept: ms };
    });

    madrpc.register('compute', (args) => {
        return { result: args.x * args.y };
    });
    "#
    .to_string()
}

/// Monte Carlo Pi test script for distributed computation testing.
pub fn monte_carlo_script() -> String {
    r#"
    madrpc.register('monte_carlo_sample', (args) => {
        const samples = args.samples || 1000000;
        const seed = args.seed || 0;

        // Simple LCG for reproducible random numbers
        let state = seed;
        const random = () => {
            state = (state * 1103515245 + 12345) & 0x7fffffff;
            return state / 0x7fffffff;
        };

        let inside = 0;
        for (let i = 0; i < samples; i++) {
            const x = random();
            const y = random();
            if (x * x + y * y <= 1) {
                inside++;
            }
        }

        return { inside, total: samples };
    });

    madrpc.register('aggregate', async (args) => {
        const numNodes = args.numNodes || 2;
        const samplesPerNode = args.samplesPerNode || 100000;

        const promises = [];
        for (let i = 0; i < numNodes; i++) {
            promises.push(madrpc.call('monte_carlo_sample', {
                samples: samplesPerNode,
                seed: i
            }));
        }

        const results = await Promise.all(promises);

        let totalInside = 0;
        let totalSamples = 0;
        for (const result of results) {
            totalInside += result.inside;
            totalSamples += result.total;
        }

        const piEstimate = 4 * totalInside / totalSamples;

        return {
            totalInside,
            totalSamples,
            piEstimate
        };
    });
    "#
    .to_string()
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
    ///
    /// The script will be copied to `/app/script.js` in the container.
    pub async fn start(script_content: String) -> anyhow::Result<Self> {
        let image = GenericImage::new("madrpc", "test")
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

    /// Start a node that registers with an orchestrator.
    pub async fn start_with_orchestrator(
        script_content: String,
        orchestrator_url: String,
    ) -> anyhow::Result<Self> {
        let image = GenericImage::new("madrpc", "test")
            .with_exposed_port(9001.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Server listening on"))
            .with_copy_to("/app/script.js", CopyDataSource::Data(script_content.into_bytes()))
            .with_cmd([
                "node",
                "-s",
                "/app/script.js",
                "-b",
                "0.0.0.0:9001",
                "--orchestrator",
                &orchestrator_url,
            ]);

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

    /// Get metrics from the node.
    pub async fn get_metrics(&self) -> anyhow::Result<serde_json::Value> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/_metrics", self.external_url))
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }
}

// ============================================================================
// Orchestrator Container
// ============================================================================

/// Wrapper for a running orchestrator container.
pub struct OrchestratorContainer {
    #[allow(dead_code)]
    container: testcontainers::ContainerAsync<GenericImage>,
    /// The host port mapped to the container's port 8080.
    pub host_port: u16,
    /// The external URL for connecting to this orchestrator.
    pub external_url: String,
}

impl OrchestratorContainer {
    /// Start a new orchestrator container with the given node URLs.
    pub async fn start(node_urls: Vec<String>) -> anyhow::Result<Self> {
        let mut cmd = vec!["orchestrator", "-b", "0.0.0.0:8080"];
        for node_url in &node_urls {
            cmd.extend(["-n", node_url.as_str()]);
        }

        let image = GenericImage::new("madrpc", "test")
            .with_exposed_port(8080.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Server listening on"))
            .with_cmd(cmd);

        let container = image.start().await?;

        let port = container.get_host_port_ipv4(8080).await?;
        let external_url = format!("http://127.0.0.1:{}", port);

        // Wait for the __health endpoint to be ready
        Self::wait_for_ready(&external_url).await?;

        Ok(Self {
            container,
            host_port: port,
            external_url,
        })
    }

    /// Wait for the orchestrator to be ready by polling the __health endpoint.
    async fn wait_for_ready(url: &str) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let health_url = format!("{}/__health", url);

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(30);

        while start.elapsed() < timeout {
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(_) | Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }

        anyhow::bail!("Orchestrator did not become ready within timeout at {}", url);
    }

    /// Get the orchestrator's URL for client connections.
    pub fn url(&self) -> &str {
        &self.external_url
    }

    /// Get metrics from the orchestrator.
    pub async fn get_metrics(&self) -> anyhow::Result<serde_json::Value> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/_metrics", self.external_url))
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }
}

// ============================================================================
// JSON-RPC Helper
// ============================================================================

/// Make a raw HTTP JSON-RPC call to the given URL.
pub async fn jsonrpc_call(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let response = client
        .post(url)
        .json(&request)
        .send()
        .await?
        .json()
        .await?;

    Ok(response)
}
