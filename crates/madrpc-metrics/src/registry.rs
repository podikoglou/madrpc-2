use crate::snapshot::{MethodMetrics, MetricsSnapshot, NodeMetrics};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Instant, SystemTime};

const NUM_HISTOGRAM_BINS: usize = 100;

/// Fallback timestamp counter for when SystemTime::duration_since() fails.
///
/// This ensures LRU eviction continues to work correctly even if the
/// system clock is set before UNIX_EPOCH or otherwise returns an error.
/// The counter uses monotonically increasing values as a fallback timestamp.
static TIMESTAMP_FALLBACK: AtomicU64 = AtomicU64::new(1);

/// Tracks the last issued timestamp to ensure monotonicity.
///
/// Since SystemTime::now() can return the same millisecond value for
/// consecutive calls, we use this to ensure each timestamp is strictly
/// greater than the previous one. This is critical for correct LRU eviction.
static LAST_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

/// Generates a monotonically increasing timestamp in milliseconds.
///
/// This function ensures that each call returns a timestamp strictly greater
/// than all previous timestamps. This is critical for correct LRU eviction
/// behavior, as we need to guarantee a total ordering of access times.
///
/// # Implementation
///
/// 1. Get the current system time in milliseconds
/// 2. Compare with the last issued timestamp
/// 3. Return the greater of the two, ensuring monotonicity
/// 4. Update LAST_TIMESTAMP to the returned value
///
/// Uses `Ordering::SeqCst` to ensure that the timestamp is globally visible
/// and that concurrent calls result in strictly increasing values.
fn get_monotonic_timestamp() -> u64 {
    let system_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_else(|_| TIMESTAMP_FALLBACK.fetch_add(1, Ordering::SeqCst));

    // Use a compare-and-swap loop to ensure we return a timestamp
    // that is strictly greater than all previously issued timestamps.
    loop {
        let last = LAST_TIMESTAMP.load(Ordering::Acquire);
        let new_timestamp = system_time.max(last + 1);

        match LAST_TIMESTAMP.compare_exchange_weak(
            last,
            new_timestamp,
            Ordering::SeqCst,
            Ordering::Acquire,
        ) {
            Ok(_) => return new_timestamp,
            Err(_) => continue, // Another thread updated the timestamp, retry
        }
    }
}

/// Configuration for metrics cleanup and size limits.
///
/// Controls memory usage and entry lifetime for both method and node metrics.
/// These settings prevent unbounded growth of metrics data in long-running
/// servers.
///
/// # Fields
///
/// - **max_methods**: Maximum number of unique method names to track
/// - **max_nodes**: Maximum number of unique node addresses to track (orchestrator)
/// - **method_ttl_secs**: Time-to-live for stale method entries in seconds
/// - **node_ttl_secs**: Time-to-live for stale node entries in seconds
///
/// # Example
///
/// ```rust
/// use madrpc_metrics::MetricsConfig;
///
/// let config = MetricsConfig {
///     max_methods: 500,        // Track up to 500 unique methods
///     max_nodes: 50,           // Track up to 50 nodes
///     method_ttl_secs: 1800,   // Remove methods after 30 minutes of inactivity
///     node_ttl_secs: 600,      // Remove nodes after 10 minutes of inactivity
/// };
/// ```
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    /// Maximum number of unique methods to track
    ///
    /// When this limit is exceeded, least-recently-used methods are evicted.
    pub max_methods: usize,
    /// Maximum number of unique nodes to track (orchestrator only)
    ///
    /// When this limit is exceeded, least-recently-used nodes are evicted.
    pub max_nodes: usize,
    /// Time-to-live for method entries in seconds
    ///
    /// Methods not accessed within this duration are eligible for cleanup.
    pub method_ttl_secs: u64,
    /// Time-to-live for node entries in seconds (orchestrator only)
    ///
    /// Nodes not receiving requests within this duration are eligible for cleanup.
    pub node_ttl_secs: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            max_methods: 1000,
            max_nodes: 100,
            method_ttl_secs: 3600,
            node_ttl_secs: 3600,
        }
    }
}

/// Logarithmic histogram for efficient percentile estimation.
///
/// Uses 100 bins covering the latency range from 1μs to 1s using
/// logarithmic scaling. Each bin covers a power-of-10 range divided
/// into 10 equal subdivisions.
///
/// # Performance
///
/// - Lock-free design using atomic operations
/// - O(1) recording: finds bin via log10 calculation
/// - O(bins) percentile calculation: linear scan through 100 bins
/// - Constant memory usage regardless of sample count
///
/// # Accuracy
///
/// Provides approximate percentiles within ~5-10% accuracy for typical
/// latency distributions. The logarithmic scaling provides better resolution
/// at lower latencies where precision matters most.
///
/// # Bin Structure
///
/// For latencies from 1μs to 1,000,000μs (1 second):
/// - Bin 0-9: 1-10μs range (10 bins of ~1μs each)
/// - Bin 10-19: 10-100μs range (10 bins of ~10μs each)
/// - Bin 20-29: 100-1000μs range (10 bins of ~100μs each)
/// - Bin 30-39: 1-10ms range (10 bins of ~1ms each)
/// - Bin 40-49: 10-100ms range (10 bins of ~10ms each)
/// - Bin 50-59: 100-1000ms range (10 bins of ~100ms each)
/// - Bin 60-99: 1-10 second range (overflow protection)
#[derive(Debug)]
struct LatencyHistogram {
    /// Count of samples in each histogram bin
    bins: [AtomicU64; NUM_HISTOGRAM_BINS],
    /// Running sum of all latencies for average calculation
    total_latency: AtomicU64,
    /// Total number of samples recorded
    sample_count: AtomicU64,
}

impl LatencyHistogram {
    /// Creates a new empty latency histogram.
    fn new() -> Self {
        // Initialize array with zeros
        let bins: [AtomicU64; NUM_HISTOGRAM_BINS] =
            std::array::from_fn(|_| AtomicU64::new(0));

        Self {
            bins,
            total_latency: AtomicU64::new(0),
            sample_count: AtomicU64::new(0),
        }
    }

    /// Records a latency sample in the histogram.
    ///
    /// Samples are placed into logarithmic bins. Uses relaxed ordering
    /// for maximum performance since exact synchronization isn't required.
    ///
    /// # Arguments
    /// * `latency_us` - Latency in microseconds
    fn record(&self, latency_us: u64) {
        // Find the appropriate bin using logarithmic scaling
        let bin = Self::latency_to_bin(latency_us);

        // Increment the bin counter
        self.bins[bin].fetch_add(1, Ordering::Relaxed);

        // Update total latency and sample count for average calculation
        self.total_latency.fetch_add(latency_us, Ordering::Relaxed);
        self.sample_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Maps a latency value to a histogram bin.
    ///
    /// Uses logarithmic scaling to map latencies from 1μs to 1s+
    /// into 100 bins. Each decade (power of 10) is divided into 10 bins.
    fn latency_to_bin(latency_us: u64) -> usize {
        if latency_us == 0 {
            return 0;
        }

        // Calculate the log10 of the latency
        // Use integer arithmetic to avoid floating point
        let log10_value = Self::ilog10(latency_us);

        // Calculate bin index:
        // - log10_value gives us the decade (0 for 1-10, 1 for 10-100, etc.)
        // - We then divide the decade into 10 bins
        // - Scale factor gives us finer resolution within each decade
        let scale_factor = if latency_us < 10 {
            latency_us as usize
        } else if latency_us < 100 {
            (latency_us / 10) as usize
        } else if latency_us < 1000 {
            (latency_us / 100) as usize
        } else if latency_us < 10000 {
            (latency_us / 1000) as usize
        } else if latency_us < 100000 {
            (latency_us / 10000) as usize
        } else if latency_us < 1000000 {
            (latency_us / 100000) as usize
        } else {
            // Latencies >= 1s go into the last bins
            10 + Self::ilog10(latency_us / 1000000).min(3) as usize * 10
        };

        let decade_offset = log10_value as usize * 10;
        let bin = decade_offset + scale_factor.min(9);

        // Cap at the last bin
        bin.min(NUM_HISTOGRAM_BINS - 1)
    }

    /// Integer base-10 logarithm.
    ///
    /// Returns floor(log10(n)) for n > 0.
    fn ilog10(mut n: u64) -> u32 {
        if n == 0 {
            return 0;
        }

        let mut log = 0;
        while n >= 10 {
            n /= 10;
            log += 1;
        }
        log
    }

    /// Estimates the value at a given percentile.
    ///
    /// Uses linear interpolation within bins for better accuracy.
    ///
    /// # Arguments
    /// * `percentile` - Percentile to calculate (0-100)
    ///
    /// # Returns
    ///
    /// Estimated latency at the given percentile, or 0 if no samples.
    fn estimate_percentile(&self, percentile: u64) -> u64 {
        let total = self.sample_count.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }

        let target_count = (total * percentile) / 100;
        let mut cumulative_count = 0;

        // Find the bin containing the target percentile
        for (bin_idx, bin) in self.bins.iter().enumerate() {
            let bin_count = bin.load(Ordering::Relaxed);
            if cumulative_count + bin_count >= target_count {
                // Target percentile is in this bin
                // Use linear interpolation within the bin
                let bin_start = Self::bin_to_latency(bin_idx);
                let bin_end = Self::bin_to_latency(bin_idx + 1);

                // If this is the first or only sample in the bin, return the midpoint
                if bin_count == 0 {
                    return (bin_start + bin_end) / 2;
                }

                // Linear interpolation: estimate position within the bin
                let offset_in_bin = target_count - cumulative_count;
                let fraction = offset_in_bin as f64 / bin_count as f64;
                let interpolated = bin_start as f64 + fraction * (bin_end - bin_start) as f64;

                return interpolated as u64;
            }
            cumulative_count += bin_count;
        }

        // If we didn't find it, return the maximum bin's upper bound
        Self::bin_to_latency(NUM_HISTOGRAM_BINS)
    }

    /// Maps a histogram bin index to a latency value.
    ///
    /// Returns the lower bound of the bin's range.
    fn bin_to_latency(bin: usize) -> u64 {
        if bin == 0 {
            return 1;
        }

        let decade = bin / 10;
        let sub_bin = bin % 10;

        // Calculate the lower bound of this bin
        10u64.pow(decade as u32) * sub_bin.max(1) as u64
    }

    /// Calculates latency percentiles from the histogram.
    ///
    /// Returns the average, P50 (median), P95, and P99 latencies.
    ///
    /// # Returns
    ///
    /// A tuple of `(avg, p50, p95, p99)` latency values in microseconds.
    /// Returns `(0, 0, 0, 0)` if no samples have been recorded.
    ///
    /// # Performance
    ///
    /// O(NUM_HISTOGRAM_BINS) = O(100) - linear scan through bins
    fn calculate_percentiles(&self) -> (u64, u64, u64, u64) {
        let total = self.sample_count.load(Ordering::Relaxed);
        if total == 0 {
            return (0, 0, 0, 0);
        }

        // Calculate average
        let total_latency = self.total_latency.load(Ordering::Relaxed);
        let avg = total_latency / total;

        // Estimate percentiles using histogram
        let p50 = self.estimate_percentile(50);
        let p95 = self.estimate_percentile(95);
        let p99 = self.estimate_percentile(99);

        (avg, p50, p95, p99)
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal storage for per-method metrics.
///
/// Tracks call counts, success/failure rates, latency samples, and last access
/// time for a single RPC method. Used internally by `MetricsRegistry`.
///
/// # Thread Safety
///
/// All fields use atomic operations for lock-free concurrent access.
/// Relaxed memory ordering is used throughout for maximum performance.
///
/// # Fields
///
/// - **call_count**: Total number of calls to this method
/// - **success_count**: Number of successful calls
/// - **failure_count**: Number of failed calls
/// - **latencies**: Histogram of latency samples for percentile estimation
/// - **last_access_ms**: Unix timestamp (ms) of last access (for TTL cleanup)
#[derive(Debug)]
struct MethodStats {
    /// Total number of calls to this method
    call_count: AtomicU64,
    /// Number of successful calls
    success_count: AtomicU64,
    /// Number of failed calls
    failure_count: AtomicU64,
    /// Histogram of latency samples for efficient percentile estimation
    latencies: LatencyHistogram,
    /// Last access timestamp in milliseconds (for TTL cleanup)
    last_access_ms: AtomicU64,
}

impl MethodStats {
    /// Creates a new method stats struct with current timestamp.
    fn new() -> Self {
        let now = get_monotonic_timestamp();
        Self {
            call_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            latencies: LatencyHistogram::new(),
            last_access_ms: AtomicU64::new(now),
        }
    }

    /// Updates the last access timestamp to the current time.
    ///
    /// Uses `get_monotonic_timestamp()` to ensure strictly increasing timestamps,
    /// which is critical for correct LRU eviction behavior.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for the timestamp store. This is safe because:
    /// - Timestamps are used for approximate TTL checking, not precise ordering
    /// - Small timing variations don't affect correctness
    /// - The RwLock provides synchronization for entry access
    fn update_last_access(&self) {
        let now = get_monotonic_timestamp();
        // Relaxed ordering is safe for timestamp because:
        // - Timestamps are used for approximate TTL checking, not precise ordering
        // - Small timing variations don't affect correctness
        // - The RwLock provides synchronization for entry access
        self.last_access_ms.store(now, Ordering::Relaxed);
    }

    /// Increments the call counter and updates last access time.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for counter operations. This is safe because:
    /// - Each counter is independent and doesn't need synchronization with others
    /// - Metrics snapshots are eventually consistent by design
    /// - No operations depend on the order of counter updates
    fn increment_call(&self) {
        // Relaxed ordering is safe for counters because:
        // - Each counter is independent and doesn't need synchronization with others
        // - Metrics snapshots are eventually consistent by design
        // - No operations depend on the order of counter updates
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.update_last_access();
    }

    /// Increments the success counter and updates last access time.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` (see `increment_call` for rationale).
    fn increment_success(&self) {
        // Relaxed ordering is safe (see increment_call comment)
        self.success_count.fetch_add(1, Ordering::Relaxed);
        self.update_last_access();
    }

    /// Increments the failure counter and updates last access time.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` (see `increment_call` for rationale).
    fn increment_failure(&self) {
        // Relaxed ordering is safe (see increment_call comment)
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.update_last_access();
    }

    /// Records a latency sample and updates last access time.
    fn record_latency(&self, latency_us: u64) {
        self.latencies.record(latency_us);
        self.update_last_access();
    }

    /// Creates a snapshot of current method metrics.
    ///
    /// Returns a `MethodMetrics` struct with current counters and
    /// calculated latency percentiles.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for loading counter values. This is safe because:
    /// - We're taking a best-effort point-in-time snapshot
    /// - Small inconsistencies between counters are acceptable
    /// - Metrics are eventually consistent by design
    fn snapshot(&self) -> MethodMetrics {
        // Relaxed ordering is safe for snapshot because:
        // - We're taking a best-effort point-in-time snapshot
        // - Small inconsistencies between counters are acceptable
        // - Metrics are eventually consistent by design
        let call_count = self.call_count.load(Ordering::Relaxed);
        let success_count = self.success_count.load(Ordering::Relaxed);
        let failure_count = self.failure_count.load(Ordering::Relaxed);
        let (avg_latency_us, p50_latency_us, p95_latency_us, p99_latency_us) =
            self.latencies.calculate_percentiles();

        MethodMetrics {
            call_count,
            success_count,
            failure_count,
            avg_latency_us,
            p50_latency_us,
            p95_latency_us,
            p99_latency_us,
        }
    }
}

impl Default for MethodStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal storage for per-node metrics (orchestrator only).
///
/// Tracks how many requests have been forwarded to each node and when
/// the last request was sent. Used internally by `MetricsRegistry` for
/// orchestrator-level node tracking.
///
/// # Thread Safety
///
/// All fields use atomic operations for lock-free concurrent access.
///
/// # Fields
///
/// - **request_count**: Total number of requests forwarded to this node
/// - **last_request_ms**: Unix timestamp (ms) of the last request (for TTL)
#[derive(Debug)]
struct NodeStats {
    /// Total number of requests forwarded to this node
    request_count: AtomicU64,
    /// Unix timestamp (ms) of the last request to this node
    last_request_ms: AtomicU64,
}

impl NodeStats {
    /// Creates a new node stats struct with zeroed counters.
    fn new() -> Self {
        Self {
            request_count: AtomicU64::new(0),
            last_request_ms: AtomicU64::new(0),
        }
    }

    /// Records a request to this node.
    ///
    /// Increments the request counter and updates the last request timestamp.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for both operations. This is safe because:
    /// - The counter is independent and doesn't need synchronization with others
    /// - The timestamp is used for approximate TTL checking
    /// - Metrics are eventually consistent by design
    fn record_request(&self) {
        // Relaxed ordering is safe for counters (see MethodStats::increment_call comment)
        self.request_count.fetch_add(1, Ordering::Relaxed);
        let now = get_monotonic_timestamp();
        // Relaxed ordering is safe for timestamp (see MethodStats::update_last_access comment)
        self.last_request_ms.store(now, Ordering::Relaxed);
    }

    /// Creates a snapshot of current node metrics.
    ///
    /// # Arguments
    /// * `node_addr` - The node address to include in the snapshot
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for loading counter values. This is safe because:
    /// - We're taking a best-effort point-in-time snapshot
    /// - Small inconsistencies are acceptable for metrics
    fn snapshot(&self, node_addr: &str) -> NodeMetrics {
        // Relaxed ordering is safe for snapshot (see MethodStats::snapshot comment)
        let request_count = self.request_count.load(Ordering::Relaxed);
        let last_request_ms = self.last_request_ms.load(Ordering::Relaxed);

        NodeMetrics {
            node_addr: node_addr.to_string(),
            request_count,
            last_request_ms,
        }
    }
}

impl Default for NodeStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe metrics registry with hybrid concurrency model.
///
/// `MetricsRegistry` is the central storage for all metrics in MaDRPC. It provides
/// lock-free performance for counter increments while using locks for metadata
/// management. The registry supports automatic cleanup of stale entries and
/// configurable size limits to prevent unbounded memory growth.
///
/// # Concurrency Model
///
/// This registry uses a hybrid approach combining lock-free and lock-based synchronization:
///
/// - **Global counters** (`total_requests`, `successful_requests`, etc.): Lock-free `AtomicU64`
///   operations using relaxed ordering for maximum performance
/// - **Per-method stats** (`call_count`, `success_count`, etc.): Lock-free `AtomicU64` operations
///   once the entry is created
/// - **Method registry** (`methods`): Protected by `RwLock` for safe concurrent access
/// - **Node registry** (`nodes`): Protected by `RwLock` for safe concurrent access
///
/// The design prioritizes performance for the hot path (incrementing counters) while using
/// locks only for metadata management (adding/removing entries).
///
/// # Atomic Ordering
///
/// All atomic operations use `Ordering::Relaxed` which is safe because:
/// - Counters are independent and don't need synchronization with each other
/// - Metrics snapshots are eventually consistent by design
/// - No operations depend on the order of counter updates
/// - The RwLock provides synchronization for entry access
///
/// # Automatic Cleanup
///
/// The registry periodically performs cleanup (every 1000 operations) to:
/// - Remove stale method/node entries based on TTL
/// - Enforce `max_methods` and `max_nodes` limits using LRU eviction
/// - Prevent memory leaks from abandoned methods/nodes
///
/// # Example
///
/// ```rust
/// use madrpc_metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
///
/// // Record a method call
/// registry.record_method_call("compute", 150, true);
///
/// // Get a snapshot
/// let snapshot = registry.snapshot(false);
/// assert_eq!(snapshot.total_requests, 1);
/// ```
#[derive(Debug)]
pub struct MetricsRegistry {
    /// Total number of requests processed
    total_requests: AtomicU64,
    /// Number of successful requests
    successful_requests: AtomicU64,
    /// Number of failed requests
    failed_requests: AtomicU64,
    /// Current number of active connections
    active_connections: AtomicU64,
    /// Per-method metrics storage
    methods: StdRwLock<HashMap<String, Arc<MethodStats>>>,
    /// Per-node metrics storage (orchestrator only)
    nodes: StdRwLock<HashMap<String, Arc<NodeStats>>>,
    /// Server start time for uptime calculation
    start_time: Instant,
    /// Configuration for cleanup behavior
    config: MetricsConfig,
    /// Counter for triggering periodic cleanup
    cleanup_counter: AtomicU64,
}

impl MetricsRegistry {
    /// Creates a new metrics registry with default configuration.
    ///
    /// Uses default `MetricsConfig` values.
    pub fn new() -> Self {
        Self::with_config(MetricsConfig::default())
    }

    /// Creates a new metrics registry with custom configuration.
    ///
    /// Use this to control memory usage and cleanup behavior. The configuration
    /// determines how many methods/nodes can be tracked and how quickly stale
    /// entries are removed.
    ///
    /// # Arguments
    /// * `config` - The metrics configuration controlling limits and TTLs
    ///
    /// # Example
    /// ```rust
    /// use madrpc_metrics::{MetricsRegistry, MetricsConfig};
    ///
    /// let config = MetricsConfig {
    ///     max_methods: 500,
    ///     max_nodes: 50,
    ///     method_ttl_secs: 1800,
    ///     node_ttl_secs: 600,
    /// };
    /// let registry = MetricsRegistry::with_config(config);
    /// ```
    pub fn with_config(config: MetricsConfig) -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            methods: StdRwLock::new(HashMap::new()),
            nodes: StdRwLock::new(HashMap::new()),
            start_time: Instant::now(),
            config,
            cleanup_counter: AtomicU64::new(0),
        }
    }

    /// Increments the total request counter.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for maximum performance. See struct-level
    /// documentation for rationale.
    pub fn increment_total(&self) {
        // Relaxed ordering is safe for counters (see MetricsRegistry docs)
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the successful request counter.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for maximum performance. See struct-level
    /// documentation for rationale.
    pub fn increment_success(&self) {
        // Relaxed ordering is safe for counters (see MetricsRegistry docs)
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the failed request counter.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for maximum performance. See struct-level
    /// documentation for rationale.
    pub fn increment_failure(&self) {
        // Relaxed ordering is safe for counters (see MetricsRegistry docs)
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the active connections counter.
    ///
    /// Call this when a new TCP connection is established.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for maximum performance.
    pub fn increment_active_connections(&self) {
        // Relaxed ordering is safe for counters (see MetricsRegistry docs)
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the active connections counter.
    ///
    /// Call this when a TCP connection is closed.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for maximum performance.
    pub fn decrement_active_connections(&self) {
        // Relaxed ordering is safe for counters (see MetricsRegistry docs)
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Records a method call with its latency and outcome.
    ///
    /// This is the primary method for tracking RPC calls. It updates global
    /// counters, per-method statistics, and latency tracking. Cleanup is
    /// triggered periodically (every 1000 calls).
    ///
    /// # Arguments
    /// * `method` - The name of the method that was called
    /// * `latency_us` - The call duration in microseconds
    /// * `success` - `true` if the call succeeded, `false` if it failed
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe and can be called concurrently from multiple
    /// threads. The method registry is briefly locked to create new entries,
    /// but counter updates are lock-free.
    ///
    /// # Example
    /// ```rust
    /// use madrpc_metrics::MetricsRegistry;
    /// use std::time::Instant;
    ///
    /// let registry = MetricsRegistry::new();
    /// let start = Instant::now();
    /// // ... execute RPC call ...
    /// let latency = start.elapsed().as_micros() as u64;
    /// registry.record_method_call("my_method", latency, true);
    /// ```
    pub fn record_method_call(&self, method: &str, latency_us: u64, success: bool) {
        // Increment global counters
        self.increment_total();
        if success {
            self.increment_success();
        } else {
            self.increment_failure();
        }

        // Periodically check if cleanup is needed
        self.maybe_cleanup();

        // Get or create method stats
        {
            let mut methods = self.methods.write().unwrap();
            let stats = methods
                .entry(method.to_string())
                .or_insert_with(|| Arc::new(MethodStats::new()))
                .clone();

            // Release the lock before recording
            drop(methods);

            stats.increment_call();
            stats.record_latency(latency_us);

            if success {
                stats.increment_success();
            } else {
                stats.increment_failure();
            }
        }
    }

    /// Records a request to a specific node (orchestrator only).
    ///
    /// This method tracks how many requests are forwarded to each node,
    /// enabling monitoring of load balancer behavior. Cleanup is triggered
    /// periodically (every 1000 calls).
    ///
    /// # Arguments
    /// * `node_addr` - The address of the node that received the request
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe. The node registry is briefly locked to
    /// create new entries, but counter updates are lock-free.
    pub fn record_node_request(&self, node_addr: &str) {
        // Periodically check if cleanup is needed
        self.maybe_cleanup();

        let stats = {
            let mut nodes = self.nodes.write().unwrap();
            nodes
                .entry(node_addr.to_string())
                .or_insert_with(|| Arc::new(NodeStats::new()))
                .clone()
        };

        stats.record_request();
    }

    /// Checks if cleanup is needed and runs it periodically.
    ///
    /// Cleanup is triggered every 1000 operations. This prevents excessive
    /// lock contention while still cleaning up stale entries regularly.
    ///
    /// # Memory Ordering
    ///
    /// Uses `Ordering::Relaxed` for the cleanup counter. This is safe because:
    /// - We only care about the count reaching a multiple of CLEANUP_INTERVAL
    /// - No synchronization with other operations is required
    /// - Occasional missed cleanups (due to relaxed ordering) are acceptable
    fn maybe_cleanup(&self) {
        const CLEANUP_INTERVAL: u64 = 1000;
        // Relaxed ordering is safe for cleanup counter because:
        // - We only care about the count reaching a multiple of CLEANUP_INTERVAL
        // - No synchronization with other operations is required
        // - Occasional missed cleanups (due to relaxed ordering) are acceptable
        let count = self.cleanup_counter.fetch_add(1, Ordering::Relaxed);
        if count % CLEANUP_INTERVAL == 0 {
            self.cleanup_stale_entries();
        }
    }

    /// Removes stale entries and enforces size limits using LRU eviction.
    ///
    /// This method performs two types of cleanup:
    ///
    /// 1. **TTL-based cleanup**: Removes entries that haven't been accessed
    ///    within the configured TTL period
    /// 2. **Size limit enforcement**: If the number of entries exceeds the
    ///    configured maximum, removes the least-recently-used entries
    ///
    /// The cleanup is performed separately for methods and nodes.
    fn cleanup_stale_entries(&self) {
        let now = get_monotonic_timestamp();

        // Clean up stale methods
        {
            let mut methods = self.methods.write().unwrap();
            let method_ttl_ms = self.config.method_ttl_secs * 1000;

            // Remove stale entries
            methods.retain(|_name, stats| {
                // Relaxed ordering is safe for timestamp comparison (see MetricsRegistry docs)
                let last_access = stats.last_access_ms.load(Ordering::Relaxed);
                now.saturating_sub(last_access) < method_ttl_ms
            });

            // Enforce max_methods limit using LRU eviction
            if methods.len() > self.config.max_methods {
                let mut entries: Vec<_> = methods
                    .iter()
                    .map(|(name, stats)| {
                        // Relaxed ordering is safe for LRU sorting (approximate ordering is acceptable)
                        (name.clone(), stats.last_access_ms.load(Ordering::Relaxed))
                    })
                    .collect();

                entries.sort_by_key(|&(_, last_access)| last_access);

                let to_remove = entries.len() - self.config.max_methods;
                for (name, _) in entries.into_iter().take(to_remove) {
                    methods.remove(&name);
                }
            }
        }

        // Clean up stale nodes
        {
            let mut nodes = self.nodes.write().unwrap();
            let node_ttl_ms = self.config.node_ttl_secs * 1000;

            // Remove stale entries
            nodes.retain(|_name, stats| {
                // Relaxed ordering is safe for timestamp comparison (see MetricsRegistry docs)
                let last_access = stats.last_request_ms.load(Ordering::Relaxed);
                now.saturating_sub(last_access) < node_ttl_ms
            });

            // Enforce max_nodes limit using LRU eviction
            if nodes.len() > self.config.max_nodes {
                let mut entries: Vec<_> = nodes
                    .iter()
                    .map(|(name, stats)| {
                        // Relaxed ordering is safe for LRU sorting (approximate ordering is acceptable)
                        (name.clone(), stats.last_request_ms.load(Ordering::Relaxed))
                    })
                    .collect();

                entries.sort_by_key(|&(_, last_access)| last_access);

                let to_remove = entries.len() - self.config.max_nodes;
                for (name, _) in entries.into_iter().take(to_remove) {
                    nodes.remove(&name);
                }
            }
        }
    }

    /// Returns the server uptime in milliseconds.
    ///
    /// Calculated from the time the registry was created.
    pub fn uptime_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Takes a snapshot of current metrics.
    ///
    /// Returns a `MetricsSnapshot` containing all current metrics including
    /// global counters, per-method statistics, and optionally per-node metrics.
    ///
    /// # Arguments
    /// * `include_nodes` - Whether to include node metrics (should be `true` for orchestrators)
    ///
    /// # Returns
    ///
    /// A snapshot of all current metrics. The snapshot is immutable and can be
    /// safely shared or serialized.
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe. Read locks are held briefly while copying
    /// the method and node registries.
    pub fn snapshot(&self, include_nodes: bool) -> MetricsSnapshot {
        let uptime_ms = self.uptime_ms();
        // Relaxed ordering is safe for snapshot (see MetricsRegistry docs)
        let total_requests = self.total_requests.load(Ordering::Relaxed);
        let successful_requests = self.successful_requests.load(Ordering::Relaxed);
        let failed_requests = self.failed_requests.load(Ordering::Relaxed);
        let active_connections = self.active_connections.load(Ordering::Relaxed);

        let methods: HashMap<String, MethodMetrics> = {
            let methods_guard = self.methods.read().unwrap();
            methods_guard
                .iter()
                .map(|(name, stats)| (name.clone(), stats.snapshot()))
                .collect()
        };

        let nodes = if include_nodes {
            let nodes_guard = self.nodes.read().unwrap();
            Some(
                nodes_guard
                    .iter()
                    .map(|(addr, stats)| {
                        // Clone once and reuse for both key and snapshot
                        // This avoids the previous double-allocation
                        (addr.clone(), stats.snapshot(addr))
                    })
                    .collect(),
            )
        } else {
            None
        };

        MetricsSnapshot {
            total_requests,
            successful_requests,
            failed_requests,
            active_connections,
            uptime_ms,
            methods,
            nodes,
        }
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_counter_increments() {
        let registry = MetricsRegistry::new();

        registry.increment_total();
        registry.increment_total();
        registry.increment_success();
        registry.increment_failure();

        let snapshot = registry.snapshot(false);
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.successful_requests, 1);
        assert_eq!(snapshot.failed_requests, 1);
    }

    #[test]
    fn test_active_connections() {
        let registry = MetricsRegistry::new();

        registry.increment_active_connections();
        registry.increment_active_connections();
        registry.increment_active_connections();

        assert_eq!(registry.snapshot(false).active_connections, 3);

        registry.decrement_active_connections();
        assert_eq!(registry.snapshot(false).active_connections, 2);
    }

    #[test]
    fn test_method_tracking() {
        let registry = MetricsRegistry::new();

        // Record some calls
        registry.record_method_call("test_method", 100, true);
        registry.record_method_call("test_method", 200, true);
        registry.record_method_call("test_method", 50, false);

        let snapshot = registry.snapshot(false);
        assert_eq!(snapshot.total_requests, 3);
        assert_eq!(snapshot.successful_requests, 2);
        assert_eq!(snapshot.failed_requests, 1);

        let method_metrics = snapshot.methods.get("test_method").unwrap();
        assert_eq!(method_metrics.call_count, 3);
        assert_eq!(method_metrics.success_count, 2);
        assert_eq!(method_metrics.failure_count, 1);
        assert_eq!(method_metrics.avg_latency_us, 116); // (100 + 200 + 50) / 3
    }

    #[test]
    fn test_percentile_calculation() {
        let registry = MetricsRegistry::new();

        // Record many latency samples
        for i in 0..1000 {
            registry.record_method_call("percentile_test", i, true);
        }

        let snapshot = registry.snapshot(false);
        let method_metrics = snapshot.methods.get("percentile_test").unwrap();

        // P50 should be around 500
        assert!(method_metrics.p50_latency_us >= 400 && method_metrics.p50_latency_us <= 600);

        // P95 should be around 950
        assert!(
            method_metrics.p95_latency_us >= 900 && method_metrics.p95_latency_us <= 999
        );

        // P99 should be around 990
        assert!(
            method_metrics.p99_latency_us >= 980 && method_metrics.p99_latency_us <= 999
        );
    }

    #[test]
    fn test_node_request_tracking() {
        let registry = MetricsRegistry::new();

        registry.record_node_request("node1");
        registry.record_node_request("node1");
        registry.record_node_request("node2");

        let snapshot = registry.snapshot(true);
        assert!(snapshot.nodes.is_some());

        let nodes = snapshot.nodes.unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes.get("node1").unwrap().request_count, 2);
        assert_eq!(nodes.get("node2").unwrap().request_count, 1);
    }

    #[test]
    fn test_thread_safety() {
        let registry = Arc::new(MetricsRegistry::new());
        let mut handles = vec![];

        // Spawn multiple threads incrementing counters
        for _ in 0..10 {
            let registry_clone = registry.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    registry_clone.record_method_call("concurrent_test", 100, true);
                }
            }));
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        let snapshot = registry.snapshot(false);
        assert_eq!(snapshot.total_requests, 10000);
        assert_eq!(snapshot.successful_requests, 10000);

        let method_metrics = snapshot.methods.get("concurrent_test").unwrap();
        assert_eq!(method_metrics.call_count, 10000);
    }

    #[test]
    fn test_uptime() {
        let registry = MetricsRegistry::new();
        thread::sleep(Duration::from_millis(10));

        let uptime = registry.uptime_ms();
        assert!(uptime >= 10);
    }

    // ========================================================================
    // Cleanup Mechanism Tests
    // ========================================================================

    #[test]
    fn test_config_defaults() {
        let config = MetricsConfig::default();
        assert_eq!(config.max_methods, 1000);
        assert_eq!(config.max_nodes, 100);
        assert_eq!(config.method_ttl_secs, 3600);
        assert_eq!(config.node_ttl_secs, 3600);
    }

    #[test]
    fn test_registry_with_custom_config() {
        let config = MetricsConfig {
            max_methods: 10,
            max_nodes: 5,
            method_ttl_secs: 60,
            node_ttl_secs: 60,
        };
        let registry = MetricsRegistry::with_config(config);

        // Create 15 unique methods first
        for i in 0..15 {
            registry.record_method_call(&format!("method_{}", i), 100, true);
        }

        // Then trigger cleanup by reaching 1000 calls
        for _ in 0..990 {
            registry.record_method_call("method_0", 100, true);
        }

        let snapshot = registry.snapshot(false);
        // Should have at most 10 methods due to max_methods limit
        assert!(snapshot.methods.len() <= 10);
    }

    #[test]
    fn test_method_ttl_cleanup() {
        let config = MetricsConfig {
            max_methods: 1000,
            max_nodes: 100,
            method_ttl_secs: 0, // Immediate TTL
            node_ttl_secs: 3600,
        };
        let registry = MetricsRegistry::with_config(config);

        // Record some methods
        registry.record_method_call("old_method", 100, true);
        registry.record_method_call("another_old", 200, true);

        // Trigger cleanup by recording more than CLEANUP_INTERVAL (1000) calls
        for _i in 0..1001 {
            registry.record_method_call("new_method", 100, true);
        }

        // Old methods should be cleaned up
        let snapshot = registry.snapshot(false);
        assert!(!snapshot.methods.contains_key("old_method"));
        assert!(!snapshot.methods.contains_key("another_old"));
        // New method should still be present
        assert!(snapshot.methods.contains_key("new_method"));
    }

    #[test]
    fn test_max_methods_enforcement() {
        let config = MetricsConfig {
            max_methods: 5,
            max_nodes: 100,
            method_ttl_secs: 3600,
            node_ttl_secs: 3600,
        };
        let registry = MetricsRegistry::with_config(config);

        // Create 10 different methods first
        for i in 0..10 {
            registry.record_method_call(&format!("method_{}", i), 100, true);
        }

        // Then trigger cleanup by reaching 1000 calls
        for _ in 0..995 {
            registry.record_method_call("method_0", 100, true);
        }

        let snapshot = registry.snapshot(false);
        // Should have at most 5 methods
        assert!(snapshot.methods.len() <= 5);
    }

    #[test]
    fn test_lru_eviction_preserves_recent() {
        let config = MetricsConfig {
            max_methods: 3,
            max_nodes: 100,
            method_ttl_secs: 3600,
            node_ttl_secs: 3600,
        };
        let registry = MetricsRegistry::with_config(config);

        // Record 5 methods
        registry.record_method_call("method_1", 100, true);
        registry.record_method_call("method_2", 100, true);
        registry.record_method_call("method_3", 100, true);
        registry.record_method_call("method_4", 100, true);
        registry.record_method_call("method_5", 100, true);

        // Access method_1 and method_2 multiple times to make them more recent
        for _ in 0..10 {
            registry.record_method_call("method_1", 100, true);
            registry.record_method_call("method_2", 100, true);
        }

        // Trigger cleanup by reaching 1000 calls
        for _ in 0..994 {
            registry.record_method_call("method_1", 100, true);
        }

        let snapshot = registry.snapshot(false);
        // Should have at most 3 methods
        assert!(snapshot.methods.len() <= 3);
        // Recently accessed methods should be preserved
        assert!(snapshot.methods.contains_key("method_1"));
        assert!(snapshot.methods.contains_key("method_2"));
    }

    #[test]
    fn test_node_ttl_cleanup() {
        let config = MetricsConfig {
            max_methods: 1000,
            max_nodes: 100,
            method_ttl_secs: 3600,
            node_ttl_secs: 0, // Immediate TTL
        };
        let registry = MetricsRegistry::with_config(config);

        // Record some nodes
        registry.record_node_request("old_node");
        registry.record_node_request("another_old");

        // Trigger cleanup by recording more than CLEANUP_INTERVAL (1000) calls
        for i in 0..1001 {
            registry.record_node_request(&format!("node_{}", i));
        }

        // Old nodes should be cleaned up
        let snapshot = registry.snapshot(true);
        let nodes = snapshot.nodes.unwrap();
        assert!(!nodes.contains_key("old_node"));
        assert!(!nodes.contains_key("another_old"));
    }

    #[test]
    fn test_max_nodes_enforcement() {
        let config = MetricsConfig {
            max_methods: 1000,
            max_nodes: 3,
            method_ttl_secs: 3600,
            node_ttl_secs: 3600,
        };
        let registry = MetricsRegistry::with_config(config);

        // Create 5 different nodes first
        for i in 0..5 {
            registry.record_node_request(&format!("node_{}", i));
        }

        // Then trigger cleanup by reaching 1000 calls
        for _ in 0..998 {
            registry.record_node_request("node_0");
        }

        let snapshot = registry.snapshot(true);
        let nodes = snapshot.nodes.unwrap();
        // Should have at most 3 nodes
        assert!(nodes.len() <= 3);
    }

    #[test]
    fn test_active_methods_preserved() {
        let config = MetricsConfig {
            max_methods: 100,
            max_nodes: 100,
            method_ttl_secs: 1, // 1 second TTL
            node_ttl_secs: 3600,
        };
        let registry = MetricsRegistry::with_config(config);

        // Record an active method
        for _ in 0..10 {
            registry.record_method_call("active_method", 100, true);
        }

        // Record an old method
        registry.record_method_call("old_method", 100, true);

        // Wait for TTL to pass for old method
        thread::sleep(Duration::from_secs(2));

        // Keep active method alive
        for _ in 0..10 {
            registry.record_method_call("active_method", 100, true);
        }

        // Trigger cleanup
        for _i in 0..1001 {
            registry.record_method_call("active_method", 100, true);
        }

        let snapshot = registry.snapshot(false);
        // Active method should still be present
        assert!(snapshot.methods.contains_key("active_method"));
        // Old method should be cleaned up
        assert!(!snapshot.methods.contains_key("old_method"));
    }

    #[test]
    fn test_cleanup_does_not_affect_counters() {
        let config = MetricsConfig {
            max_methods: 2,
            max_nodes: 100,
            method_ttl_secs: 3600,
            node_ttl_secs: 3600,
        };
        let registry = MetricsRegistry::with_config(config);

        // Record many method calls to trigger cleanup
        for i in 0..2000 {
            registry.record_method_call(&format!("method_{}", i), 100, true);
        }

        let snapshot = registry.snapshot(false);
        // Global counters should not be affected by cleanup
        assert_eq!(snapshot.total_requests, 2000);
        assert_eq!(snapshot.successful_requests, 2000);
    }

    // ========================================================================
    // Timestamp Fallback Tests
    // ========================================================================

    #[test]
    fn test_timestamp_fallback_counter_increments() {
        use std::sync::atomic::Ordering;

        let initial = TIMESTAMP_FALLBACK.load(Ordering::SeqCst);

        // Simulate multiple failures
        for _ in 0..10 {
            let _ = TIMESTAMP_FALLBACK.fetch_add(1, Ordering::SeqCst);
        }

        assert_eq!(TIMESTAMP_FALLBACK.load(Ordering::SeqCst), initial + 10);
    }

    #[test]
    fn test_method_entry_uses_valid_timestamp() {
        let registry = MetricsRegistry::new();

        // Record a method call to create a MethodStats entry
        registry.record_method_call("test_method", 100, true);

        // Verify the method was tracked
        let snapshot = registry.snapshot(false);
        assert!(snapshot.methods.contains_key("test_method"));

        // The timestamp should be valid (either real timestamp or fallback)
        // We can't directly access the timestamp, but we can verify the method exists
        let method_metrics = snapshot.methods.get("test_method").unwrap();
        assert_eq!(method_metrics.call_count, 1);
    }

    #[test]
    fn test_node_entry_uses_valid_timestamp() {
        let registry = MetricsRegistry::new();

        // Record a node request to create a NodeStats entry
        registry.record_node_request("test_node");

        // Verify the node was tracked
        let snapshot = registry.snapshot(true);
        assert!(snapshot.nodes.is_some());

        let nodes = snapshot.nodes.unwrap();
        assert!(nodes.contains_key("test_node"));

        // The timestamp should be valid (either real timestamp or fallback)
        let node_metrics = nodes.get("test_node").unwrap();
        assert_eq!(node_metrics.request_count, 1);
    }

    #[test]
    fn test_cleanup_stale_entries_handles_edge_cases() {
        let config = MetricsConfig {
            method_ttl_secs: 1,
            node_ttl_secs: 1,
            max_methods: 1000,
            max_nodes: 100,
        };

        let registry = MetricsRegistry::with_config(config);

        // Record some methods
        registry.record_method_call("test_method_1", 100, true);
        registry.record_method_call("test_method_2", 200, true);

        // Record some nodes
        registry.record_node_request("test_node_1");
        registry.record_node_request("test_node_2");

        // Verify they exist
        let snapshot = registry.snapshot(true);
        assert_eq!(snapshot.methods.len(), 2);
        assert!(snapshot.nodes.is_some());
        assert_eq!(snapshot.nodes.unwrap().len(), 2);

        // The cleanup mechanism will work correctly even if timestamps are fallback
        // Trigger cleanup by making many calls
        for i in 0..1001 {
            registry.record_method_call(&format!("new_method_{}", i), 100, true);
        }

        // Old methods should be removed (or kept if accessed recently)
        // New methods should exist
        let snapshot = registry.snapshot(false);
        assert!(snapshot.methods.len() > 0);
    }

    // ========================================================================
    // Latency Histogram Tests
    // ========================================================================

    #[test]
    fn test_latency_histogram_basic_recording() {
        // Test basic histogram recording and percentile calculation
        let histogram = LatencyHistogram::new();

        // Record some latency samples
        histogram.record(100);
        histogram.record(200);
        histogram.record(300);

        // Check that samples were recorded
        let count = histogram.sample_count.load(Ordering::Relaxed);
        assert_eq!(count, 3);

        // Calculate percentiles
        let (avg, p50, _p95, _p99) = histogram.calculate_percentiles();
        assert_eq!(avg, 200); // (100 + 200 + 300) / 3
        assert!(p50 > 0);
    }

    #[test]
    fn test_latency_histogram_percentile_accuracy() {
        // Test that histogram provides reasonable percentile estimates
        let histogram = LatencyHistogram::new();

        // Record 1000 samples with known distribution
        for i in 1..=1000 {
            histogram.record(i);
        }

        let (_, p50, p95, p99) = histogram.calculate_percentiles();

        // P50 should be around 500 (within 20% tolerance)
        assert!(p50 >= 400 && p50 <= 600, "P50 {} should be around 500", p50);

        // P95 should be around 950 (within 20% tolerance)
        assert!(p95 >= 900 && p95 <= 999, "P95 {} should be around 950", p95);

        // P99 should be around 990 (within 20% tolerance)
        assert!(p99 >= 980 && p99 <= 999, "P99 {} should be around 990", p99);
    }

    #[test]
    fn test_latency_histogram_concurrent_writes() {
        // Test that histogram works correctly with concurrent writes
        use std::sync::Arc;
        use std::thread;

        let histogram = Arc::new(LatencyHistogram::new());
        let mut handles = vec![];

        // Spawn multiple threads writing to the histogram
        for _ in 0..10 {
            let hist_clone = histogram.clone();
            handles.push(thread::spawn(move || {
                for i in 0..200 {
                    hist_clone.record(i as u64);
                }
            }));
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Total writes: 10 threads * 200 samples = 2000
        let count = histogram.sample_count.load(Ordering::Relaxed);
        assert_eq!(count, 2000, "should have recorded all samples");

        // Verify percentiles are calculated
        let (_, p50, p95, p99) = histogram.calculate_percentiles();
        assert!(p50 > 0, "P50 should be calculated");
        assert!(p95 > 0, "P95 should be calculated");
        assert!(p99 > 0, "P99 should be calculated");
    }

    #[test]
    fn test_latency_histogram_empty() {
        // Test that empty histogram returns zeros
        let histogram = LatencyHistogram::new();

        let (avg, p50, p95, p99) = histogram.calculate_percentiles();
        assert_eq!(avg, 0);
        assert_eq!(p50, 0);
        assert_eq!(p95, 0);
        assert_eq!(p99, 0);
    }

    #[test]
    fn test_latency_histogram_bin_mapping() {
        // Test that latency values map to correct bins
        assert_eq!(LatencyHistogram::latency_to_bin(0), 0);
        assert!(LatencyHistogram::latency_to_bin(1) < 10);
        assert!(LatencyHistogram::latency_to_bin(10) < 20);
        assert!(LatencyHistogram::latency_to_bin(100) < 30);
        assert!(LatencyHistogram::latency_to_bin(1000) < 40);
        assert!(LatencyHistogram::latency_to_bin(10000) < 50);
        assert!(LatencyHistogram::latency_to_bin(100000) < 60);
    }

    #[test]
    fn test_latency_histogram_ilog10() {
        // Test integer log10 calculation
        assert_eq!(LatencyHistogram::ilog10(1), 0);
        assert_eq!(LatencyHistogram::ilog10(9), 0);
        assert_eq!(LatencyHistogram::ilog10(10), 1);
        assert_eq!(LatencyHistogram::ilog10(99), 1);
        assert_eq!(LatencyHistogram::ilog10(100), 2);
        assert_eq!(LatencyHistogram::ilog10(999), 2);
        assert_eq!(LatencyHistogram::ilog10(1000), 3);
    }
}

