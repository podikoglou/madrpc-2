use thiserror::Error;

#[derive(Error, Debug)]
pub enum MadrpcError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] postcard::Error),

    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    #[error("QUIC connection error: {0}")]
    QuicConnection(String),

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

// Implement From for QUIC-specific errors
impl From<quinn::ConnectError> for MadrpcError {
    fn from(err: quinn::ConnectError) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

impl From<quinn::ConnectionError> for MadrpcError {
    fn from(err: quinn::ConnectionError) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

impl From<quinn::ReadExactError> for MadrpcError {
    fn from(err: quinn::ReadExactError) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

impl From<std::net::AddrParseError> for MadrpcError {
    fn from(err: std::net::AddrParseError) -> Self {
        MadrpcError::InvalidRequest(err.to_string())
    }
}

impl From<rustls::Error> for MadrpcError {
    fn from(err: rustls::Error) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

impl From<quinn::WriteError> for MadrpcError {
    fn from(err: quinn::WriteError) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

impl From<quinn::ClosedStream> for MadrpcError {
    fn from(err: quinn::ClosedStream) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

impl From<quinn::crypto::rustls::NoInitialCipherSuite> for MadrpcError {
    fn from(err: quinn::crypto::rustls::NoInitialCipherSuite) -> Self {
        MadrpcError::Connection(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MadrpcError>;
