//! MaDRPC Common Types and Transport
//!
//! This crate provides the core protocol definitions and TCP transport layer
//! for the MaDRPC distributed RPC system.
//!
//! # Overview
//!
//! MaDRPC (Massively Distributed RPC) is a Rust-based distributed RPC system
//! that enables parallel computation across multiple nodes. This crate contains
//! the shared protocol and transport infrastructure used by all components:
//!
//! - **Protocol Layer**: Request/Response types, error handling, and type definitions
//! - **Transport Layer**: TCP-based communication with JSON serialization
//!
//! # Architecture
//!
//! The system uses a simple wire protocol:
//! - **Transport**: TCP with keep-alive connections
//! - **Serialization**: JSON
//! - **Message Format**: `[4-byte length prefix as u32 big-endian] + [JSON data]`
//! - **Max Message Size**: 100 MB (prevents memory exhaustion)
//!
//! # Components
//!
//! - [`protocol`] - Core protocol types (Request, Response, Error)
//! - [`transport`] - TCP transport and codec implementations
//!
//! # Example
//!
//! ```no_run
//! use madrpc_common::{Request, Response, MadrpcError};
//! use serde_json::json;
//!
//! // Create a request
//! let request = Request::new("compute", json!({"n": 1000}))
//!     .with_timeout(5000);
//!
//! // Process and create response
//! let response = Response::success(request.id, json!({"result": 42}));
//! ```

pub mod protocol;
pub mod transport;

pub use protocol::*;
