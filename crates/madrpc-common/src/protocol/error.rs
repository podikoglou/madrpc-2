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
}

impl From<std::net::AddrParseError> for MadrpcError {
    fn from(err: std::net::AddrParseError) -> Self {
        MadrpcError::InvalidRequest(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MadrpcError>;
