//! MaDRPC Transport Layer
//!
//! This module provides TCP transport and codecs for sending/receiving RPC messages.
//!
//! # Architecture
//!
//! The transport layer handles all network communication using:
//! - **Transport**: TCP with keep-alive connections
//! - **Codec**: JSON serialization for protocol messages
//! - **Wire Format**: `[4-byte length prefix as u32 big-endian] + [JSON data]`
//!
//! # Components
//!
//! - **[`Codec`]** / **[`JsonCodec`]**: Encode/decode protocol messages to JSON
//! - **[`TcpTransport`]**: Synchronous TCP transport (used by nodes)
//! - **[`TcpTransportAsync`]**: Async TCP transport (used by orchestrator)
//! - **[`TcpServer`]**: Async TCP server (used by orchestrator)
//!
//! # Message Size Limits
//!
//! All transport implementations enforce a maximum message size of 100 MB
//! to prevent memory exhaustion attacks.
//!
//! # Example
//!
//! ```no_run
//! use madrpc_common::transport::TcpTransport;
//! use madrpc_common::protocol::Request;
//! use serde_json::json;
//!
//! // Connect to a node
//! let transport = TcpTransport::new().unwrap();
//! let mut stream = transport.connect("127.0.0.1:8080").unwrap();
//!
//! // Send a request
//! let request = Request::new("compute", json!({"n": 100}));
//! let response = transport.send_request(&mut stream, &request).unwrap();
//! ```

pub mod codec;
pub mod http;
pub mod tcp;
pub mod tcp_server;

pub use codec::{Codec, JsonCodec};
pub use http::{HttpTransport, HyperRequest, HyperResponse};
pub use tcp::{TcpTransport, TcpTransportAsync};
pub use tcp_server::TcpServer;

#[cfg(test)]
mod tests;
