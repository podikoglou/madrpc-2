//! Re-exports of built-in procedure types from madrpc-common.

pub use madrpc_common::protocol::builtin::{
    ServerType,
    MethodMetrics,
    NodeMetrics,
    MetricsSnapshot,
    ServerBase,
};

// Legacy re-exports for backward compatibility
/// Legacy alias for ServerBase - use ServerBase from madrpc_common instead
pub type ServerInfo = ServerBase;
