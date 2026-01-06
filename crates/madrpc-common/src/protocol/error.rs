use thiserror::Error;

#[derive(Error, Debug)]
pub enum MadrpcError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    #[error("MessagePack serialization error: {0}")]
    MessagePackSerialization(String),

    #[error("Request timeout after {0}ms")]
    Timeout(u64),

    #[error("Node unavailable: {0}")]
    NodeUnavailable(String),

    #[error("JavaScript execution error: {0}")]
    JavaScriptExecution(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("All nodes failed")]
    AllNodesFailed,

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Pool acquisition timeout after {0}ms")]
    PoolTimeout(u64),

    #[error("Connection pool exhausted for {0}")]
    PoolExhausted(String),
}

impl MadrpcError {
    /// Check if this error is retryable
    ///
    /// Returns true for transient errors like network issues, timeouts, and unavailable nodes.
    /// Returns false for permanent errors like invalid requests or JavaScript execution errors.
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
    fn from(err: std::net::AddrParseError) -> Self {
        MadrpcError::InvalidRequest(err.to_string())
    }
}

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
