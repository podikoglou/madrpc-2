use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

pub type RequestId = u64;
pub type MethodName = String;
pub type RpcArgs = serde_json::Value;

static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: RequestId,
    pub method: MethodName,
    pub args: RpcArgs,
    pub timeout_ms: Option<u64>,
}

impl Request {
    pub fn new(method: impl Into<String>, args: RpcArgs) -> Self {
        Request {
            id: generate_request_id(),
            method: method.into(),
            args,
            timeout_ms: None,
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

fn generate_request_id() -> RequestId {
    // Try to use system time as the base
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // Always increment the counter to ensure uniqueness
    let counter = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

    // Combine timestamp and counter to ensure uniqueness
    // Use the lower 32 bits for counter and upper 32 bits for timestamp
    (timestamp & 0xFFFFFFFF00000000) | (counter & 0xFFFFFFFF)
}
