# MaDRPC - Complete LLM Reference Document

## Project Overview

**MaDRPC** (Massively Distributed RPC) is a Rust-based distributed RPC system enabling massively parallel JavaScript computation across multiple nodes using the **Boa JavaScript Engine** and **HTTP/JSON-RPC 2.0** transport.

**Architecture:** Three-tier system
- **Nodes** (`madrpc-server`) - Execute JavaScript functions using Boa
- **Orchestrator** (`madrpc-orchestrator`) - Round-robin load balancer with circuit breaker
- **Client** (`madrpc-client`) - HTTP client with automatic retry logic

**Key Properties:**
- Fresh Boa Context per request (true parallelism, no shared JavaScript state)
- HTTP/1.1 with keep-alive (hyper connection pooling)
- JSON-RPC 2.0 wire protocol
- Async/await with tokio multi-threaded runtime

---

## Crate Structure

```
madrpc-2/
├── crates/
│   ├── madrpc-common/       # Protocol, transport, auth, rate limiting
│   ├── madrpc-server/       # Node: JavaScript execution
│   ├── madrpc-orchestrator/ # Load balancer with circuit breaker
│   ├── madrpc-client/       # RPC client with retry logic
│   ├── madrpc-metrics/      # Metrics collection infrastructure
│   └── madrpc-cli/          # Command-line interface and TUI
├── examples/
│   ├── monte-carlo-pi/      # Monte Carlo Pi estimation
│   └── simple-test/         # Basic RPC test
└── Cargo.toml               # Workspace configuration
```

---

# 1. madrpc-common - Protocol & Transport Layer

**Purpose:** Single source of truth for MaDRPC wire protocol and shared infrastructure.

**Dependencies:** hyper 1.5, tokio 1.0, serde 1.0, serde_json 1.0, thiserror 1.0, tracing 0.1, http-body-util 0.1

**Module Structure:**
```
src/
├── lib.rs              # Public exports
├── auth.rs             # API key authentication
├── rate_limit.rs       # Token bucket rate limiting
├── protocol/
│   ├── mod.rs          # Protocol exports
│   ├── error.rs        # MadrpcError enum
│   ├── jsonrpc.rs      # JSON-RPC 2.0 types
│   ├── requests.rs     # Request type
│   ├── responses.rs    # Response type
│   └── builtin.rs      # Built-in procedure types
└── transport/
    ├── mod.rs          # Transport exports
    └── http.rs         # HTTP utilities
```

## Core Type Aliases

```rust
pub type RequestId = u64;           // Unique request identifier
pub type MethodName = String;       // Method name
pub type RpcArgs = serde_json::Value;    // Arguments
pub type RpcResult = serde_json::Value;  // Result
pub type Result<T> = std::result::Result<T, MadrpcError>;
```

## Request Type (requests.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: RequestId,              // timestamp << 32 | counter
    pub method: MethodName,
    pub args: RpcArgs,
    pub timeout_ms: Option<u64>,
    pub idempotency_key: Option<String>,
}

impl Request {
    pub fn new(method: impl Into<String>, args: RpcArgs) -> Self;
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self;
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self;
}
```

**Request ID Generation:** Combines UNIX timestamp (upper 32 bits) with atomic counter (lower 32 bits) using `Ordering::Relaxed`.

## Response Type (responses.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: RequestId,
    pub result: Option<RpcResult>,
    pub error: Option<String>,
    pub success: bool,
}

impl Response {
    pub fn success(id: RequestId, result: RpcResult) -> Self;
    pub fn error(id: RequestId, error: impl Into<String>) -> Self;
}
```

## Error Type (error.rs)

```rust
#[derive(ThisError, Debug)]
pub enum MadrpcError {
    #[error("Transport error: {0}")]
    Transport(String),                    // retryable

    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),  // non-retryable

    #[error("Request timeout after {0}ms")]
    Timeout(u64),                         // retryable

    #[error("Node unavailable: {0}")]
    NodeUnavailable(String),              // retryable

    #[error("JavaScript execution error: {0}")]
    JavaScriptExecution(String),          // non-retryable

    #[error("Invalid response: {0}")]
    InvalidResponse(String),              // non-retryable

    #[error("All nodes failed")]
    AllNodesFailed,                       // non-retryable

    #[error("Invalid request: {0}")]
    InvalidRequest(String),               // non-retryable

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),           // retryable

    #[error("Connection error: {0}")]
    Connection(String),                   // retryable

    #[error("Pool acquisition timeout after {0}ms")]
    PoolTimeout(u64),                     // retryable

    #[error("Connection pool exhausted for {0}")]
    PoolExhausted(String),                // non-retryable

    #[error("Payload size {0} bytes exceeds maximum allowed size of {1} bytes")]
    PayloadTooLarge(usize, usize),        // non-retryable
}

impl MadrpcError {
    pub fn is_retryable(&self) -> bool;
}
```

## JSON-RPC 2.0 Types (jsonrpc.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,       // Must be "2.0"
    pub method: String,
    pub params: Value,
    pub id: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
    pub id: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

// Standard error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;
pub const REQUEST_TOO_LARGE: i32 = -32001;
pub const NODE_REGISTRATION_ERROR: i32 = -32002;

impl JsonRpcError {
    pub fn parse_error() -> Self;
    pub fn invalid_request() -> Self;
    pub fn method_not_found() -> Self;
    pub fn invalid_params(msg: &str) -> Self;
    pub fn internal_error(msg: &str) -> Self;
    pub fn server_error(msg: &str) -> Self;
    pub fn request_too_large(limit: usize) -> Self;
    pub fn node_registration_error(msg: &str) -> Self;
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self;
    pub fn error(id: Value, error: JsonRpcError) -> Self;
}
```

## Built-in Procedure Types (builtin.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerType {
    Node,
    Orchestrator,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: String,  // "healthy"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMetrics {
    pub call_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_latency_us: u64,
    pub p50_latency_us: u64,
    pub p95_latency_us: u64,
    pub p99_latency_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetrics {
    pub node_addr: String,
    pub request_count: u64,
    pub last_request_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub active_connections: u64,
    pub uptime_ms: u64,
    pub methods: HashMap<String, MethodMetrics>,
    pub nodes: Option<HashMap<String, NodeMetrics>>,
}

pub type MetricsResponse = MetricsSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerBase {
    pub server_type: ServerType,
    pub version: String,     // From CARGO_PKG_VERSION
    pub uptime_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub version: String,
    pub uptime_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorNodeInfo {
    pub addr: String,
    pub enabled: bool,
    pub disable_reason: Option<String>,
    pub circuit_state: String,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorInfo {
    pub version: String,
    pub uptime_ms: u64,
    pub total_nodes: usize,
    pub enabled_nodes: usize,
    pub disabled_nodes: usize,
    pub nodes: Vec<OrchestratorNodeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "server_type", rename_all = "lowercase")]
pub enum InfoResponse {
    Node(NodeInfo),
    Orchestrator(OrchestratorInfo),
}

// Node registration types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistrationRequest {
    pub node_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistrationResponse {
    pub registered_url: String,
    pub is_new_registration: bool,
}
```

## Authentication (auth.rs)

```rust
#[derive(Clone, Debug)]
pub struct AuthConfig {
    api_key: Option<String>,
}

impl AuthConfig {
    pub fn with_api_key(api_key: impl Into<String>) -> Self;
    pub fn disabled() -> Self;
    pub fn requires_auth(&self) -> bool;
    pub fn validate_api_key(&self, provided_key: &str) -> bool;
}

impl Default for AuthConfig {
    fn default() -> Self;  // Returns disabled()
}

pub fn extract_api_key(header_value: Option<&str>) -> Option<&str>;
```

**Security:** Constant-time string comparison prevents timing attacks.

## Rate Limiting (rate_limit.rs)

```rust
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub requests_per_second: f64,
    pub burst_size: u32,
    pub cleanup_interval: Duration,
    pub entry_ttl: Duration,
}

impl RateLimitConfig {
    pub fn new(requests_per_second: f64, burst_size: u32) -> Self;
    pub fn per_second(rps: f64) -> Self;
    pub fn per_minute(rpm: u32) -> Self;
}

impl Default for RateLimitConfig {
    fn default() -> Self;  // Returns disabled
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateLimitResult {
    Allowed,
    RateLimited { retry_after: Duration },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool;
    pub fn retry_after(&self) -> Option<Duration>;
}

// Token bucket (private)
struct TokenBucket {
    tokens: f64,
    last_update: Instant,
}

#[derive(Clone)]
pub struct RateLimiter {
    pub config: RateLimitConfig,
    buckets: Arc<RwLock<HashMap<IpAddr, TokenBucket>>>,
    last_cleanup: Arc<RwLock<Instant>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self;
    pub fn disabled() -> Self;
    pub async fn check_rate_limit(&self, ip: &IpAddr) -> RateLimitResult;
    pub async fn tracked_ip_count(&self) -> usize;
    pub fn is_enabled(&self) -> bool;
}
```

**Algorithm:** Token bucket with per-IP tracking. Automatic cleanup of stale entries (default 5-minute TTL).

## HTTP Transport (transport/http.rs)

```rust
pub const MAX_PAYLOAD_SIZE: usize = 10 * 1024 * 1024;  // 10 MB

pub type HyperRequest = Request<Incoming>;
pub type HyperResponse = Response<Full<Bytes>>;

pub struct HttpTransport;

impl HttpTransport {
    pub fn parse_jsonrpc(body: Bytes) -> Result<JsonRpcRequest, MadrpcError>;
    pub fn to_http_response(jsonrpc: JsonRpcResponse) -> HyperResponse;
    pub fn to_http_error(id: serde_json::Value, error: JsonRpcError) -> HyperResponse;
    pub fn build_request(
        method: &str,
        params: serde_json::Value,
        id: serde_json::Value,
    ) -> Result<JsonRpcRequest, MadrpcError>;
    pub fn to_http_response_with_status(
        jsonrpc: JsonRpcResponse,
        status: StatusCode
    ) -> HyperResponse;
}
```

**Security:** Payload size validated before JSON parsing (10 MB limit).

---

# 2. madrpc-server - Node Implementation

**Purpose:** Execute JavaScript functions using Boa JavaScript Engine with thread-safe concurrent execution.

**Dependencies:** boa_engine 0.21, boa_gc 0.21, tokio 1.x, hyper 1.5, hyper-util 0.1, http-body-util 0.1, serde_json 1.0, thiserror 1.0, tracing 0.1, futures-concurrency 7.6, futures-lite 2.5, num_cpus 1.16, madrpc-client, madrpc-common, madrpc-metrics

**Module Structure:**
```
src/
├── lib.rs              # Public exports
├── node.rs             # Core Node struct
├── http_router.rs      # JSON-RPC routing
├── http_server.rs      # HTTP server with security
├── resource_limits.rs  # Execution limits
└── runtime/
    ├── mod.rs          # Runtime exports
    ├── context.rs      # MadrpcContext wrapper
    ├── bindings.rs     # JavaScript native functions
    ├── conversions.rs  # JSON ↔ JS conversion
    ├── job_executor.rs # Tokio job queue integration
    └── tests.rs        # Integration tests
```

## ResourceLimits

```rust
pub struct ResourceLimits {
    pub execution_timeout: Duration,
}

impl ResourceLimits {
    pub fn new() -> Self;
    pub fn with_execution_timeout(mut self, timeout: Duration) -> Self;
    pub fn validate(&self) -> Result<(), String>;
}

impl Default for ResourceLimits {
    fn default() -> Self {
        execution_timeout: Duration::from_secs(30),  // 30 seconds
    }
}
```

**Validation Rules:** Execution timeout must be > 0 and ≤ 1 hour.

## Node (node.rs)

```rust
pub struct Node {
    script_path: PathBuf,
    script_source: Arc<String>,
    metrics_collector: Arc<NodeMetricsCollector>,
    orchestrator_client: Option<Arc<madrpc_client::MadrpcClient>>,
    resource_limits: ResourceLimits,
}

impl Node {
    // Constructors
    pub fn new(script_path: PathBuf) -> Result<Self>;
    pub fn with_resource_limits(script_path: PathBuf, resource_limits: ResourceLimits) -> Result<Self>;
    pub fn with_orchestrator(script_path: PathBuf, orchestrator_addr: String) -> Result<Self>;
    pub fn with_orchestrator_and_resource_limits(
        script_path: PathBuf,
        orchestrator_addr: String,
        resource_limits: ResourceLimits,
    ) -> Result<Self>;

    // Builder pattern
    pub fn and_resource_limits(mut self, resource_limits: ResourceLimits) -> Self;
    pub fn resource_limits(&self) -> &ResourceLimits;

    // Request handling
    pub fn handle_request(&self, request: &Request) -> Result<Response>;
    pub async fn call_rpc(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value>;

    // Metadata
    pub fn script_path(&self) -> &PathBuf;
    pub async fn get_metrics(&self) -> Result<MetricsResponse>;
    pub async fn get_info(&self) -> Result<InfoResponse>;

    // Orchestrator registration
    pub async fn register_with_orchestrator(&self, public_url: String) -> Result<()>;
}
```

**Key Design:**
- Script source cached as `Arc<String>` (avoid file I/O)
- Fresh Boa Context per request (true parallelism)
- `call_rpc` uses `tokio::time::timeout` with `spawn_blocking`
- Orchestrator registration: exponential backoff (max 5 attempts, starting 500ms)

## NodeRouter (http_router.rs)

```rust
pub struct NodeRouter {
    node: Arc<Node>,
}

impl NodeRouter {
    pub fn new(node: Arc<Node>) -> Self;
    pub async fn handle_request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse, JsonRpcError>;
}
```

**Routing Logic:**
- `_health` → `HealthResponse::healthy()`
- `_metrics` → `node.get_metrics()`
- `_info` → `node.get_info()`
- Other → `node.call_rpc()`

## HttpServer (http_server.rs)

```rust
pub struct HttpServer {
    router: Arc<NodeRouter>,
    connection_semaphore: Arc<Semaphore>,
    auth_config: AuthConfig,
    rate_limiter: Arc<RateLimiter>,
}

const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;  // 10 MB
const MAX_CONCURRENT_CONNECTIONS: usize = 1000;

impl HttpServer {
    pub fn new(node: Arc<Node>) -> Self;
    pub fn with_auth(mut self, auth_config: AuthConfig) -> Self;
    pub fn with_rate_limit(mut self, rate_limit_config: RateLimitConfig) -> Self;
    pub async fn run(self, addr: SocketAddr) -> Result<(), MadrpcError>;
}
```

**Security Features:** API key auth, rate limiting, connection limiting (semaphore), body size limit.

## MadrpcContext (runtime/context.rs)

```rust
pub struct MadrpcContext {
    _thread_marker: ThreadNotSendSync,  // PhantomData<Rc<()>>
    ctx: boa_engine::context::Context,
    client: Option<Arc<madrpc_client::MadrpcClient>>,
    job_executor: Rc<TokioJobExecutor>,
}

struct ThreadNotSendSync {
    _marker: PhantomData<Rc<()>>,  // Makes !Send and !Sync
}

const MAX_PROMISE_WAIT_MS: u64 = 30_000;
const PROMISE_POLL_INTERVAL_ASYNC_MS: u64 = 10;
const PROMISE_POLL_INTERVAL_SYNC_MS: u64 = 50;

impl MadrpcContext {
    pub fn new(script_path: impl AsRef<Path>) -> Result<Self>;
    pub fn from_source(script_source: &str) -> Result<Self>;
    pub fn with_client(script_path: impl AsRef<Path>, client: Option<MadrpcClient>) -> Result<Self>;
    pub fn with_client_from_source(script_source: &str, client: Option<MadrpcClient>) -> Result<Self>;

    pub fn call_rpc(&mut self, method: &str, args: JsonValue) -> Result<JsonValue>;
    pub async fn call_rpc_async(&mut self, method: &str, args: JsonValue) -> Result<JsonValue>;
    pub async fn distributed_call(&self, method: &str, args: JsonValue) -> Result<JsonValue>;
}
```

**Thread Safety:** `!Send` and `!Sync` enforced at type level via `PhantomData<Rc<()>>`. Boa Context has thread-local state.

## TokioJobExecutor (runtime/job_executor.rs)

```rust
pub struct TokioJobExecutor {
    promise_jobs: RefCell<VecDeque<PromiseJob>>,
    async_jobs: RefCell<VecDeque<NativeAsyncJob>>,
    generic_jobs: RefCell<VecDeque<GenericJob>>,
}

impl TokioJobExecutor {
    pub fn new() -> Self;
    pub fn has_pending_jobs(&self) -> bool;
}

impl JobExecutor for TokioJobExecutor {
    fn enqueue_job(self: Rc<Self>, job: Job, _context: &mut Context);
    fn run_jobs(self: Rc<Self>, context: &mut Context) -> JsResult<()>;
    async fn run_jobs_async(self: Rc<Self>, context: &RefCell<&mut Context>) -> JsResult<()>;
}
```

**Execution Algorithm (async):**
1. Poll all async jobs concurrently using `FutureGroup`
2. Wait for at least one to complete
3. Drain promise and generic jobs (microtasks)
4. Yield back to tokio
5. Repeat until empty

## JavaScript Bindings (runtime/bindings.rs)

```rust
struct ClientWrapper(Arc<madrpc_client::MadrpcClient>);

unsafe impl Trace for ClientWrapper {
    custom_trace!(this, _mark, {});  // Arc doesn't need GC
}

static BLOCKING_RUNTIME: OnceLock<Mutex<tokio::runtime::Runtime>> = OnceLock::new();

pub(crate) fn install_madrpc_bindings(
    ctx: &mut Context,
    client: Option<Arc<madrpc_client::MadrpcClient>>,
) -> Result<()>;

pub fn get_blocking_runtime() -> std::io::Result<&'static Mutex<tokio::runtime::Runtime>>;

async fn rpc_call_async(
    client: Arc<madrpc_client::MadrpcClient>,
    method: String,
    json_args: serde_json::Value,
) -> std::result::Result<serde_json::Value, String>;
```

**JavaScript API:**
```javascript
madrpc.register(name, function)     // Register function
madrpc.call(method, args)           // Async distributed RPC (returns Promise)
madrpc.callSync(method, args)       // Synchronous blocking RPC
```

**Security:** No pointer-as-number storage. Client stored via closure capture with Arc. Shared blocking runtime reused.

## JSON Conversions (runtime/conversions.rs)

```rust
pub fn json_to_js_value(json: JsonValue, ctx: &mut Context) -> Result<JsValue>;
pub fn js_value_to_json(value: JsValue, ctx: &mut Context) -> Result<JsonValue>;
```

**Type Mapping:**
| JSON | JavaScript |
|------|------------|
| null | null |
| boolean | Boolean |
| number | Number |
| string | String |
| array | Array |
| object | Object |

**Special Cases:** JavaScript `undefined` → JSON `null`, Symbol keys skipped.

## Script Execution Model

```rust
// Each request:
let script_source = self.script_source.clone();  // Arc<String>
let mut ctx = MadrpcContext::from_source(&script_source)?;
let result = ctx.call_rpc_async(method, params).await?;
```

**Why Fresh Context:**
- Boa's string interner is per-context
- True parallelism (no shared state)
- Thread-safe by design
- Prevents memory leaks

**Promise Handling:**
- **Sync (`call_rpc`)**: 50ms poll interval, blocking
- **Async (`call_rpc_async`)**: 10ms poll interval, tokio-aware
- **Timeout**: 30 seconds maximum

---

# 3. madrpc-orchestrator - Load Balancer

**Purpose:** Round-robin load balancer with circuit breaker and health checking.

**Dependencies:** tokio 1.x, hyper 1.5, hyper-util 0.1, http-body-util 0.1, tower 0.5, tower-http 0.6, madrpc-client, madrpc-common, madrpc-metrics

**Module Structure:**
```
src/
├── lib.rs              # Public exports
├── orchestrator.rs     # Main orchestrator with retry
├── load_balancer.rs    # Round-robin with circuit breaker
├── node.rs             # Node state management
├── health_checker.rs   # Periodic health checking
├── http_router.rs      # JSON-RPC routing
└── http_server.rs      # HTTP server with auth/rate limiting
```

## Node State Management (node.rs)

```rust
pub enum DisableReason {
    Manual,        // User-initiated, never auto-re-enabled
    HealthCheck,   // Auto-disabled, can be auto-re-enabled
}

pub enum HealthCheckStatus {
    Healthy,
    Unhealthy(String),
}

pub enum CircuitBreakerState {
    Closed,    // Normal operation
    Open,      // Failing fast
    HalfOpen,  // Testing recovery
}

// State transitions:
// Closed → Open: failures >= threshold
// Open → HalfOpen: exponential backoff timeout
// HalfOpen → Closed: successful health check
// HalfOpen → Open: failed health check

pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,      // Default: 5
    pub base_timeout_secs: u64,       // Default: 30
    pub max_timeout_secs: u64,        // Default: 300
    pub backoff_multiplier: f64,      // Default: 2.0
}

impl CircuitBreakerConfig {
    pub fn calculate_timeout(&self, consecutive_failures: u32) -> Duration;
}
// Formula: min(base * multiplier^(failures-1), max)
// Example (base=30s, max=300s, mult=2.0):
//   1st: 30s, 2nd: 60s, 3rd: 120s, 4th: 240s, 5th+: 300s

pub struct Node {
    pub addr: String,
    pub enabled: bool,
    pub disable_reason: Option<DisableReason>,
    pub consecutive_failures: u32,
    pub last_health_check: Option<Instant>,
    pub last_health_check_status: Option<HealthCheckStatus>,
    pub circuit_state: CircuitBreakerState,
    pub circuit_opened_at: Option<SystemTime>,
    pub request_count: u64,
    pub last_request_time: Option<SystemTime>,
}

impl Node {
    pub fn new(addr: String) -> Self;
    pub fn should_attempt_half_open(&self, config: &CircuitBreakerConfig) -> bool;
    pub fn transition_circuit_state(&mut self, new_state: CircuitBreakerState);
}
```

## LoadBalancer (load_balancer.rs)

```rust
pub struct LoadBalancer {
    nodes: HashMap<String, Node>,
    enabled_nodes: Vec<String>,
    round_robin_index: usize,
    circuit_config: CircuitBreakerConfig,
}

impl LoadBalancer {
    pub fn new(node_addrs: Vec<String>) -> Self;
    pub fn with_config(node_addrs: Vec<String>, circuit_config: CircuitBreakerConfig) -> Self;

    // Core selection algorithm
    pub fn next_node(&mut self) -> Option<String>;
    // Algorithm:
    // 1. Return None if no enabled nodes
    // 2. Iterate from current index
    // 3. Skip Open circuits
    // 4. Return first Closed/HalfOpen
    // 5. Return None if all Open
    // 6. Always advance round_robin_index

    // Node management
    pub fn disable_node(&mut self, addr: &str) -> bool;     // Manual
    pub fn enable_node(&mut self, addr: &str) -> bool;
    pub fn auto_disable_node(&mut self, addr: &str) -> bool; // Health check
    pub fn auto_enable_node(&mut self, addr: &str) -> bool;
    pub fn add_node(&mut self, node_addr: String);
    pub fn remove_node(&mut self, node_addr: &str);

    // Health updates
    pub fn update_health_status(&mut self, addr: &str, status: HealthCheckStatus);
    pub fn check_circuit_timeouts(&mut self) -> bool;  // Open → HalfOpen

    // Queries
    pub fn circuit_state(&self, addr: &str) -> Option<CircuitBreakerState>;
    pub fn open_circuit_nodes(&self) -> Vec<String>;
    pub fn half_open_circuit_nodes(&self) -> Vec<String>;
    pub fn consecutive_failures(&self, addr: &str) -> u32;
    pub fn all_nodes(&self) -> Vec<Node>;
    pub fn enabled_nodes(&self) -> Vec<String>;        // O(1)
    pub fn disabled_nodes(&self) -> Vec<String>;       // O(N)
    pub fn node_count(&self) -> usize;
    pub fn nodes(&self) -> Vec<String>;
    pub fn node_metrics(&self) -> HashMap<String, madrpc_metrics::NodeMetrics>;
}
```

**Complexity:** `next_node` is O(N) worst case (all circuits open), O(1) best case.

## HealthChecker (health_checker.rs)

```rust
pub struct HealthCheckConfig {
    pub interval: Duration,      // Default: 5 seconds
    pub timeout: Duration,       // Default: 2000ms
    pub failure_threshold: u32,  // Default: 3
}

pub struct HealthCheckUpdate {
    pub node_addr: String,
    pub status: HealthCheckStatus,
    pub should_enable: bool,
    pub should_disable: bool,
}

pub struct HealthChecker {
    load_balancer: Arc<RwLock<LoadBalancer>>,
    config: HealthCheckConfig,
}

impl HealthChecker {
    pub fn new(load_balancer: Arc<RwLock<LoadBalancer>>, config: HealthCheckConfig) -> MadrpcResult<Self>;
    pub fn spawn(self) -> tokio::task::JoinHandle<()>;

    async fn run(self);  // Main loop, never returns
    async fn check_all_nodes(&self);
    async fn check_node_health(addr: &str, timeout: Duration) -> MadrpcResult<()>;
    async fn process_health_result(&self, node: Node, result: MadrpcResult<()>) -> HealthCheckUpdate;
    async fn apply_health_update(&self, update: HealthCheckUpdate);
}
```

**Health Check Flow:**
1. Check circuit timeouts (Open → HalfOpen)
2. Fetch all nodes
3. Spawn parallel checks via `futures::join_all`
4. Process results
5. Apply updates atomically

**Auto-Enable/Disable Logic:**
- Manual disables never affected by health checks
- Health check disables only affect auto-disabled nodes

## Orchestrator (orchestrator.rs)

```rust
pub struct RetryConfig {
    pub max_retries: usize,           // Default: 3
    pub initial_backoff_ms: u64,      // Default: 50ms
    pub max_backoff_ms: u64,          // Default: 5000ms
    pub backoff_multiplier: f64,      // Default: 2.0
}
// Backoff sequence: 50ms, 100ms, 200ms, ... (capped at 5000ms)

pub struct Orchestrator {
    load_balancer: Arc<RwLock<LoadBalancer>>,
    metrics_collector: Arc<OrchestratorMetricsCollector>,
    retry_config: RetryConfig,
    _health_checker_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Orchestrator {
    pub fn new(node_addrs: Vec<String>) -> Result<Self>;
    pub fn with_config(node_addrs: Vec<String>, health_config: HealthCheckConfig) -> Result<Self>;
    pub fn with_retry_config(
        node_addrs: Vec<String>,
        health_config: HealthCheckConfig,
        retry_config: RetryConfig
    ) -> Result<Self>;

    // Core methods
    pub async fn forward_request(&self, request: &Request) -> Result<Response>;
    pub async fn forward_request_jsonrpc(&self, req: JsonRpcRequest) -> Result<serde_json::Value>;
    async fn send_http_request(&self, node_addr: &str, method: &str, params: &Value) -> Result<Value>;

    // Retry logic
    fn is_retryable(&self, error: &MadrpcError) -> bool;
    pub fn is_retryable_static(error: &MadrpcError) -> bool;
    // Retryable: AllNodesFailed, NodeUnavailable
    // Non-retryable: InvalidRequest, JavaScriptExecution, Transport

    // Node management
    pub async fn add_node(&self, node_addr: String);
    pub async fn remove_node(&self, node_addr: &str);
    pub async fn disable_node(&self, node_addr: &str) -> bool;
    pub async fn enable_node(&self, node_addr: &str) -> bool;

    // Queries
    pub async fn nodes_with_status(&self) -> Vec<Node>;
    pub async fn node_count(&self) -> usize;
    pub async fn nodes(&self) -> Vec<String>;
    pub async fn get_metrics(&self) -> Result<MetricsResponse>;
    pub async fn get_info(&self) -> Result<InfoResponse>;
    pub async fn register_node(&self, node_url: String) -> Result<NodeRegistrationResponse>;
}
```

**Request Strategy:** HTTP for all node communication. Each request creates fresh client (true parallelism via hyper connection pooling).

## OrchestratorRouter (http_router.rs)

```rust
pub struct OrchestratorRouter {
    orchestrator: Arc<Orchestrator>,
}

impl OrchestratorRouter {
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self;
    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse;

    async fn handle_metrics(&self, req: JsonRpcRequest) -> JsonRpcResponse;
    async fn handle_info(&self, req: JsonRpcRequest) -> JsonRpcResponse;
    async fn handle_register(&self, req: JsonRpcRequest) -> JsonRpcResponse;
    async fn forward_to_node(&self, req: JsonRpcRequest) -> JsonRpcResponse;
}
```

**Routing:**
- `_metrics` → Local orchestrator metrics
- `_info` → Local orchestrator info
- `_register` → Register new node
- Other → Forward via load balancer

## HttpServer (http_server.rs)

```rust
pub struct HttpServer {
    router: Arc<OrchestratorRouter>,
    auth_config: AuthConfig,
    rate_limiter: Arc<RateLimiter>,
}

impl HttpServer {
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self;
    pub fn with_auth(mut self, auth_config: AuthConfig) -> Self;
    pub fn with_rate_limit(mut self, rate_limit_config: RateLimitConfig) -> Self;
    pub async fn run(self, addr: SocketAddr) -> Result<(), MadrpcError>;
}
```

**Endpoints:** POST / (JSON-RPC), GET /__health (health check)

**Pipeline:** Rate limit → Auth → Content-Type → Parse → Route → Return

---

# 4. madrpc-client - RPC Client

**Purpose:** HTTP client for making JSON-RPC calls with automatic retry logic.

**Dependencies:** madrpc-common, tokio 1.x, thiserror 1.0, serde_json 1.0, rand 0.8, tracing 0.1, hyper 1.5, hyper-util 0.1, http-body-util 0.1, futures 0.3

**Module Structure:**
```
src/
├── lib.rs          # Public exports
└── client.rs       # Main implementation
tests/
└── http_client_test.rs
```

## Public API

```rust
pub mod client;
pub use client::{MadrpcClient, RetryConfig, RetryConfigError, ConnectionConfig};
```

## MadrpcClient

```rust
pub struct MadrpcClient {
    base_url: String,
    http_client: Arc<Client<HttpConnector, Full<Bytes>>>,
    retry_config: RetryConfig,
}

impl MadrpcClient {
    pub fn new(base_url: impl Into<String>) -> MadrpcResult<Self>;
    pub fn with_configs(
        base_url: impl Into<String>,
        retry_config: RetryConfig,
        connection_config: ConnectionConfig,
    ) -> MadrpcResult<Self>;
    pub fn with_connection_config(
        base_url: impl Into<String>,
        connection_config: ConnectionConfig,
    ) -> MadrpcResult<Self>;
    pub fn with_retry_config(
        base_url: impl Into<String>,
        retry_config: RetryConfig,
    ) -> MadrpcResult<Self>;

    #[must_use]
    pub fn with_retry(mut self, retry_config: RetryConfig) -> Self;

    pub async fn call(&self, method: impl Into<String>, params: Value) -> MadrpcResult<Value>;

    fn build_http_client(config: &ConnectionConfig) -> Client<HttpConnector, Full<Bytes>>;
    async fn try_call(&self, request: &JsonRpcRequest) -> MadrpcResult<Value>;
}

impl Clone for MadrpcClient {}  // Shares underlying HTTP client
```

## RetryConfig

```rust
pub struct RetryConfig {
    pub max_attempts: u32,        // >= 1
    pub base_delay_ms: u64,       // >= 0
    pub max_delay_ms: u64,        // >= base_delay_ms
    pub backoff_multiplier: f64,  // > 0
}

impl RetryConfig {
    pub fn new(
        max_attempts: u32,
        base_delay_ms: u64,
        max_delay_ms: u64,
        backoff_multiplier: f64,
    ) -> Result<Self, RetryConfigError>;
    pub fn validate(&self) -> Result<(), RetryConfigError>;
    fn calculate_delay(&self, attempt: u32) -> Duration;
}
// Formula: delay_ms = min(base * mult^(attempt-1), max) + jitter(0..delay*0.1)

impl Default for RetryConfig {
    fn default() -> Self {
        max_attempts: 3,
        base_delay_ms: 100,
        max_delay_ms: 5000,
        backoff_multiplier: 2.0,
    }
}
```

## RetryConfigError

```rust
pub enum RetryConfigError {
    InvalidMaxAttempts(u32),
    InvalidBackoffMultiplier(f64),
    InvalidBaseDelay(i64),
    InvalidMaxDelay(i64),
    InvalidDelayRange { max_delay: u64, base_delay: u64 },
}
```

## ConnectionConfig

```rust
pub struct ConnectionConfig {
    pub pool_idle_timeout_secs: u64,       // Default: 90
    pub keep_alive_secs: Option<u64>,
    pub max_idle_per_host: Option<usize>,
}

impl ConnectionConfig {
    pub fn new(
        pool_idle_timeout_secs: u64,
        keep_alive_secs: Option<u64>,
        max_idle_per_host: Option<usize>,
    ) -> Self;
    pub fn long_lived() -> Self;   // 5 min timeout, 100 max idle
    pub fn short_lived() -> Self;  // 10 sec timeout, 5 max idle
}

impl Default for ConnectionConfig {
    fn default() -> Self;
}

impl Clone for ConnectionConfig {}
```

## Retry Algorithm

```
FOR attempt in 1..=max_attempts:
    1. Log retry attempt (if attempt > 1)
    2. Execute try_call()
    3. IF success: return result
    4. IF error:
        a. IF NOT retryable: return error immediately
        b. IF retryable AND NOT last attempt:
            - Calculate delay (exponential backoff + jitter)
            - Sleep for delay
    5. Continue

IF all attempts exhausted:
    Return last error or "All retry attempts exhausted"
```

**Delay Calculation:** `delay_ms = min(base * mult^(attempt-1), max) + random(0..delay*0.1)`

**Example (base=100, max=5000, mult=2.0):**
- Attempt 1: 100ms ± 10ms
- Attempt 2: 200ms ± 20ms
- Attempt 3: 400ms ± 40ms
- Attempt 4: 800ms ± 80ms
- ...capped at 5000ms

## Error Classification

**Retryable (transient):**
- `Transport(String)`
- `Timeout(u64)`
- `NodeUnavailable(String)`
- `Connection(String)`
- `Io(std::io::Error)`
- HTTP 5xx
- JSON-RPC Internal Error (-32603)
- JSON-RPC Server Errors (-32099 to -32001)

**Non-Retryable (permanent):**
- `InvalidRequest(String)`
- `InvalidResponse(String)`
- `JavaScriptExecution(String)`
- `AllNodesFailed`
- HTTP 4xx
- JSON-RPC Parse/Invalid/MethodNotFound/InvalidParams
- JSON-RPC application errors (positive codes)

---

# 5. madrpc-metrics - Metrics Infrastructure

**Purpose:** High-performance, thread-safe metrics collection.

**Dependencies:** madrpc-common, serde 1.0, serde_json 1.0, atomic 0.6

**Module Structure:**
```
src/
├── lib.rs          # Public exports
├── collector.rs    # MetricsCollector trait
├── registry.rs     # MetricsRegistry
└── snapshot.rs     # Re-exports from madrpc-common
```

## Public API

```rust
pub use collector::{MetricsCollector, NodeMetricsCollector, OrchestratorMetricsCollector};
pub use registry::{MetricsConfig, MetricsRegistry};
pub use snapshot::{MethodMetrics, MetricsSnapshot, NodeMetrics, ServerInfo, ServerType};
```

## MetricsCollector Trait

```rust
pub trait MetricsCollector: Send + Sync {
    fn is_metrics_request(&self, method: &str) -> bool;
    fn handle_metrics_request(&self, method: &str, id: u64) -> MadrpcResult<Response>;
    fn record_call(&self, method: &str, start_time: Instant, success: bool);
    fn snapshot(&self) -> MetricsSnapshot;
}
```

## NodeMetricsCollector

```rust
pub struct NodeMetricsCollector {
    registry: Arc<MetricsRegistry>,
}

impl NodeMetricsCollector {
    pub fn new() -> Self;
    pub fn with_config(config: MetricsConfig) -> Self;
    pub fn with_registry(registry: Arc<MetricsRegistry>) -> Self;
}

impl MetricsCollector for NodeMetricsCollector {
    fn is_metrics_request(&self, method: &str) -> bool {
        method == "_metrics" || method == "_info"
    }
    // ...
}
```

## OrchestratorMetricsCollector

```rust
pub struct OrchestratorMetricsCollector {
    registry: Arc<MetricsRegistry>,
}

impl OrchestratorMetricsCollector {
    pub fn new() -> Self;
    pub fn with_config(config: MetricsConfig) -> Self;
    pub fn with_registry(registry: Arc<MetricsRegistry>) -> Self;
    pub fn record_node_request(&self, node_addr: &str);
}

impl MetricsCollector for OrchestratorMetricsCollector {
    fn snapshot(&self) -> MetricsSnapshot {
        self.registry.snapshot(true)  // Include nodes
    }
}
```

## MetricsConfig

```rust
pub struct MetricsConfig {
    pub max_methods: usize,      // Default: 1000
    pub max_nodes: usize,        // Default: 100
    pub method_ttl_secs: u64,    // Default: 3600 (1 hour)
    pub node_ttl_secs: u64,      // Default: 3600
}
```

## MetricsRegistry

```rust
pub struct MetricsRegistry {
    // Lock-free atomics
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    active_connections: AtomicU64,

    // RwLock-protected
    methods: StdRwLock<HashMap<String, Arc<MethodStats>>>,
    nodes: StdRwLock<HashMap<String, Arc<NodeStats>>>,

    start_time: Instant,
    config: MetricsConfig,
    cleanup_counter: AtomicU64,
}

impl MetricsRegistry {
    pub fn new() -> Self;
    pub fn with_config(config: MetricsConfig) -> Self;

    // Counters (all use Ordering::Relaxed)
    pub fn increment_total(&self);
    pub fn increment_success(&self);
    pub fn increment_failure(&self);
    pub fn increment_active_connections(&self);
    pub fn decrement_active_connections(&self);

    // Recording
    pub fn record_method_call(&self, method: &str, latency_us: u64, success: bool);
    pub fn record_node_request(&self, node_addr: &str);

    // Utility
    pub fn uptime_ms(&self) -> u64;
    pub fn snapshot(&self, include_nodes: bool) -> MetricsSnapshot;
}
```

## Private Structs (registry.rs)

```rust
struct LatencyBuffer {
    samples: Vec<AtomicU64>,  // 1000-element circular buffer
    index: AtomicU64,
    count: AtomicU64,         // Capped at 1000
}

struct MethodStats {
    call_count: AtomicU64,
    success_count: AtomicU64,
    failure_count: AtomicU64,
    latencies: LatencyBuffer,
    last_access_ms: AtomicU64,
}

struct NodeStats {
    request_count: AtomicU64,
    last_request_ms: AtomicU64,
}
```

## Thread Safety Model

**Hybrid Concurrency:**
1. **Hot Path (Counters):** Lock-free `AtomicU64` with `Ordering::Relaxed`
2. **Cold Path (Metadata):** `StdRwLock` for HashMap access

**Memory Ordering Rationale:** Counters are independent; small inconsistencies acceptable; RwLock synchronizes entry access.

## Monotonic Timestamp Generation

```rust
static TIMESTAMP_FALLBACK: AtomicU64 = AtomicU64::new(1);
static LAST_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

fn get_monotonic_timestamp() -> u64 {
    let system_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_else(|_| TIMESTAMP_FALLBACK.fetch_add(1, Ordering::SeqCst));

    loop {
        let last = LAST_TIMESTAMP.load(Ordering::Acquire);
        let new_timestamp = system_time.max(last + 1);

        match LAST_TIMESTAMP.compare_exchange_weak(
            last, new_timestamp,
            Ordering::SeqCst,
            Ordering::Acquire,
        ) {
            Ok(_) => return new_timestamp,
            Err(_) => continue,
        }
    }
}
```

**Purpose:** Strictly increasing timestamps for LRU eviction, with fallback for clock errors.

## Automatic Cleanup

```rust
const CLEANUP_INTERVAL: u64 = 1000;

fn maybe_cleanup(&self) {
    let count = self.cleanup_counter.fetch_add(1, Ordering::Relaxed);
    if count % CLEANUP_INTERVAL == 0 {
        self.cleanup_stale_entries();
    }
}
```

**Cleanup Strategy:**
1. TTL-based: Remove entries not accessed within TTL
2. LRU eviction: Enforce max_methods/max_nodes limits

---

# 6. madrpc-cli - Command Line Interface

**Purpose:** CLI entry point with commands for all components and real-time metrics TUI.

**Dependencies:** argh 0.1, tokio 1.x, ratatui 0.29, crossterm 0.28, anyhow 1.0, madrpc-server, madrpc-orchestrator, madrpc-client

**Module Structure:**
```
src/
├── main.rs         # CLI entry point
├── top.rs          # TUI implementation
└── lib.rs          # Public exports
tests/
└── cli_integration_test.rs
```

## Commands

```rust
pub enum Commands {
    Node(NodeArgs),
    Orchestrator(OrchestratorArgs),
    Top(TopArgs),
    Call(CallArgs),
}
```

## NodeArgs

```rust
struct NodeArgs {
    #[argh(option, short = 's')]
    script: String,                   // Required

    #[argh(option, short = 'b', default = "\"127.0.0.1:9001\".into()")]
    bind: String,

    #[argh(option, long = "orchestrator")]
    orchestrator: Option<String>,

    #[argh(option, long = "public-url")]
    public_url: Option<String>,

    #[argh(option, long = "max-execution-time", default = "30000")]
    max_execution_time_ms: u64,

    #[argh(option, long = "api-key")]
    api_key: Option<String>,

    #[argh(option, long = "rate-limit-rps")]
    rate_limit_rps: Option<f64>,
}
```

## OrchestratorArgs

```rust
struct OrchestratorArgs {
    #[argh(option, short = 'b', default = "\"0.0.0.0:8080\".into()")]
    bind: String,

    #[argh(option, short = 'n')]
    nodes: Vec<String>,               // Multiple --node flags

    #[argh(option, long = "health-check-interval", default = "5")]
    health_check_interval_secs: u64,

    #[argh(option, long = "health-check-timeout", default = "2000")]
    health_check_timeout_ms: u64,

    #[argh(option, long = "health-check-failure-threshold", default = "3")]
    health_check_failure_threshold: u32,

    #[argh(switch, long = "disable-health-check")]
    disable_health_check: bool,

    #[argh(option, long = "api-key")]
    api_key: Option<String>,

    #[argh(option, long = "rate-limit-rps")]
    rate_limit_rps: Option<f64>,
}
```

## TopArgs

```rust
struct TopArgs {
    #[argh(positional)]
    server_address: String,

    #[argh(option, short = 'i', long = "interval", default = "250")]
    interval_ms: u64,
}
```

## CallArgs

```rust
struct CallArgs {
    #[argh(positional)]
    server_address: String,

    #[argh(positional)]
    method: String,

    #[argh(option, short = 'a', long = "args", default = "\"{}\".into()")]
    args: String,
}
```

## TUI Implementation (top.rs)

```rust
struct TopApp {
    server_address: String,
    server_type: Option<ServerType>,
    current_metrics: Option<MetricsSnapshot>,
    error_message: Option<String>,
    last_update: Option<Instant>,
    interval_ms: u64,
    should_quit: bool,
}

pub async fn run_top(server_address: String, interval_ms: u64) -> Result<()>;
```

**TUI Architecture:**
- **ratatui**: Terminal UI framework
- **crossterm**: Cross-platform terminal control
- **tokio**: Async metrics fetching

**TerminalGuard:** RAII pattern for restoring terminal state on drop.

**Layout:** Three-section (title bar, summary, content area)

**Drawing Methods:**
- `draw_title_bar()` - Logo, server info, quit hint
- `draw_summary()` - Requests, success rate, uptime
- `draw_error()` - Error display
- `draw_nodes_table()` - Node distribution (orchestrator only)
- `draw_methods_table()` - Method metrics with latencies

## URL Validation

```rust
fn validate_http_url(url: &str, description: &str) -> Result<()> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Invalid {}: '{}' must start with http:// or https://",
            description, url
        ))
    }
}
```

**Validation Points:** All orchestrator/node URLs and server addresses must have http:// or https:// prefix.

## Logging Strategy

```rust
// No logging for call and top commands
if !matches!(cli.command, Commands::Call(_) | Commands::Top(_)) {
    if std::env::var("RUST_LOG").as_deref() != Ok("off") {
        // Initialize tracing
    }
}
```

**Rules:**
- `call`/`top`: No logging (clean output)
- `node`/`orchestrator`: Full logging (INFO default)
- `RUST_LOG=off`: Suppress all logging

---

# Build and Run Commands

```bash
# Build the project
cargo build --release

# Start an orchestrator
cargo run --bin madrpc -- orchestrator \
  -b 0.0.0.0:8080 \
  -n http://127.0.0.1:9001 \
  -n http://127.0.0.1:9002

# Start a compute node
cargo run --bin madrpc -- node \
  -s examples/monte-carlo-pi/scripts/pi.js \
  -b 0.0.0.0:9001

# Monitor with real-time metrics TUI
cargo run --bin madrpc -- top http://127.0.0.1:8080

# Run examples
cargo run -p monte-carlo-pi
cargo run -p simple-test
```

---

# Key Design Patterns

## Per-Request Context Isolation (madrpc-server)

**Pattern:** Each HTTP request creates a fresh `MadrpcContext`.

**Benefits:**
- True parallelism (no shared mutable state)
- Thread safety by construction
- Memory isolation
- Simplified reasoning

## "Stupid Forwarder" Orchestrator (madrpc-orchestrator)

**Pattern:** Orchestrator does NOT execute JavaScript, only forwards requests.

**Rationale:**
- Clear separation of concerns
- Easy horizontal scaling
- Nodes handle all computation

## Circuit Breaker Integration (madrpc-orchestrator)

**Pattern:** Skip Open circuits without attempting connection.

**Benefits:**
- Fast fail behavior
- Prevents cascading failures
- Clear recovery path via HalfOpen

## Closure Capture for Client Storage (madrpc-server)

**Pattern:** Store `Arc<MadrpcClient>` in closure captures instead of pointer-as-number.

**Security:**
- Eliminates unsafe pointer storage
- Proper reference counting
- Type-safe lifetime management

## Script Source Caching (madrpc-server)

**Pattern:** Cache script source as `Arc<String>`, parse per request.

**Rationale:**
- Boa's string interner is per-context
- Can't share parsed AST across contexts
- File I/O expensive, parsing relatively cheap

---

# Performance Characteristics

## Concurrency Model

**Nodes/Orchestrator:**
- tokio multi-threaded runtime
- One task per connection
- Non-blocking I/O

**Client:**
- Connection pooling via hyper
- Exponential backoff with jitter
- Concurrent requests supported

## Memory Usage

**Per-Request Allocation:**
- Fresh Boa Context (~100KB-1MB)
- Script source shared via `Arc`
- Temporary values during conversion

## Complexity

**LoadBalancer.next_node:** O(N) worst case, O(1) best case
**RateLimiter check:** O(1) hash map lookup
**Metrics recording:** O(1) amortized

---

# Security Considerations

## Input Validation
- Body size limit: 10 MB
- Only POST accepted for JSON-RPC
- Method name validated

## Authentication
- API key via `X-API-Key` header
- Constant-time comparison
- Optional (disabled by default)

## Rate Limiting
- Token bucket per-IP
- Automatic stale entry cleanup
- Optional (disabled by default)

## Resource Limits
- Execution timeout: 30s default, 1h max
- Promise timeout: 30s
- Connection limit: 1000 concurrent

## Code Injection Prevention
- No `eval()` on user input
- Only registered functions callable
- Arguments passed as structured data

---

# Wire Protocol

**Transport:** HTTP/1.1
**Content-Type:** `application/json`
**Protocol:** JSON-RPC 2.0

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "method_name",
  "params": { ... },
  "id": 1
}
```

**Success Response:**
```json
{
  "jsonrpc": "2.0",
  "result": { ... },
  "error": null,
  "id": 1
}
```

**Error Response:**
```json
{
  "jsonrpc": "2.0",
  "result": null,
  "error": {
    "code": -32601,
    "message": "Method not found",
    "data": null
  },
  "id": 1
}
```

**Built-in Procedures:**
- `_health` - Health check
- `_metrics` - Performance metrics
- `_info` - Server information
- `_register` - Node registration (orchestrator)

---

# JavaScript Integration

## Registration Pattern

```javascript
madrpc.register('method_name', (args) => {
    const value = args.parameter || 0;
    // ... computation
    return { result: value };
});
```

## Distributed RPC Calls

```javascript
madrpc.register('aggregate', async (args) => {
    const promises = [];
    for (let i = 0; i < args.numNodes; i++) {
        promises.push(madrpc.call('remote_method', {
            samples: args.samplesPerNode,
            seed: i
        }));
    }
    const results = await Promise.all(promises);
    // ... aggregation
    return { aggregated: results };
});
```

## Async Call

```javascript
madrpc.call('remote_method', {arg: 42}).then(result => {
    console.log(result);
});
```

## Sync Call

```javascript
const result = madrpc.callSync('remote_method', {arg: 42});
console.log(result);
```

---

This document provides a complete, LLM-optimized reference for the MaDRPC project.
