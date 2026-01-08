//! MaDRPC Error Types
//!
//! This module defines the comprehensive error type for the MaDRPC system,
//! with classification for retryable vs non-retryable errors.

use thiserror::Error as ThisError;

/// MaDRPC error type representing all possible errors in the system.
///
/// This enum captures all error conditions that can occur in the MaDRPC system,
/// from network issues to JavaScript execution errors. Each error variant
/// includes context information for debugging and logging.
///
/// # Error Classification
///
/// Errors are classified as **retryable** or **non-retryable** via the
/// [`is_retryable()`](Self::is_retryable) method:
///
/// - **Retryable**: Transport, Timeout, NodeUnavailable, Io, Connection, PoolTimeout
/// - **Non-retryable**: JavaScriptExecution, InvalidResponse, AllNodesFailed,
///   InvalidRequest, PoolExhausted
#[derive(ThisError, Debug)]
pub enum MadrpcError {
    /// Low-level transport error (e.g., socket read/write failures)
    #[error("Transport error: {0}")]
    Transport(String),

    /// JSON serialization or deserialization error
    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    /// Request timeout after the specified duration
    #[error("Request timeout after {0}ms")]
    Timeout(u64),

    /// Node is unavailable or unreachable
    #[error("Node unavailable: {0}")]
    NodeUnavailable(String),

    /// JavaScript execution error on the compute node
    #[error("JavaScript execution error: {0}")]
    JavaScriptExecution(String),

    /// Invalid response received from a node
    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// All nodes in the pool failed to process the request
    #[error("All nodes failed")]
    AllNodesFailed,

    /// Invalid request format or parameters
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Standard IO error (file, network, etc.)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// TCP connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Timeout while acquiring a connection from the pool
    #[error("Pool acquisition timeout after {0}ms")]
    PoolTimeout(u64),

    /// Connection pool exhausted (all connections in use)
    #[error("Connection pool exhausted for {0}")]
    PoolExhausted(String),
}

impl MadrpcError {
    /// Check if this error is retryable
    ///
    /// Returns `true` for transient errors that may succeed on retry, such as:
    /// - Network issues (Transport, Io, Connection)
    /// - Timeouts (Timeout, PoolTimeout)
    /// - Node unavailability
    ///
    /// Returns `false` for permanent errors that will not succeed on retry, such as:
    /// - Invalid requests or responses
    /// - JavaScript execution errors
    /// - All nodes failed
    /// - Pool exhaustion
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::protocol::error::MadrpcError;
    ///
    /// let timeout_err = MadrpcError::Timeout(5000);
    /// assert!(timeout_err.is_retryable());
    ///
    /// let invalid_err = MadrpcError::InvalidRequest("bad data".to_string());
    /// assert!(!invalid_err.is_retryable());
    /// ```
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            MadrpcError::Transport(_)
                | MadrpcError::Timeout(_)
                | MadrpcError::NodeUnavailable(_)
                | MadrpcError::Io(_)
                | MadrpcError::Connection(_)
                | MadrpcError::PoolTimeout(_)
        )
    }
}

impl From<std::net::AddrParseError> for MadrpcError {
    /// Converts address parse errors to `InvalidRequest` errors
    fn from(err: std::net::AddrParseError) -> Self {
        MadrpcError::InvalidRequest(err.to_string())
    }
}

impl From<tokio::task::JoinError> for MadrpcError {
    /// Converts tokio task join errors to `Transport` errors
    fn from(err: tokio::task::JoinError) -> Self {
        MadrpcError::Transport(format!("Task join error: {}", err))
    }
}

/// Result type alias for MaDRPC operations
///
/// This is a convenience alias for `std::result::Result<T, MadrpcError>`
/// used throughout the MaDRPC codebase.
pub type Result<T> = std::result::Result<T, MadrpcError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_timeout_error() {
        let error = MadrpcError::PoolTimeout(5000);
        assert_eq!(error.to_string(), "Pool acquisition timeout after 5000ms");
        assert!(error.is_retryable());
    }

    #[test]
    fn test_pool_exhausted_error() {
        let error = MadrpcError::PoolExhausted("localhost:8080".to_string());
        assert_eq!(error.to_string(), "Connection pool exhausted for localhost:8080");
        // PoolExhausted is not retryable - it's a permanent state
        assert!(!error.is_retryable());
    }

    #[test]
    fn test_pool_timeout_is_retryable() {
        let error = MadrpcError::PoolTimeout(1000);
        assert!(error.is_retryable());
    }
}
