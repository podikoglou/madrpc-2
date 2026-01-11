//! MaDRPC Common Types and Transport
//!
//! This crate provides the core protocol definitions and HTTP transport layer
//! for the MaDRPC distributed RPC system.
//!
//! # Overview
//!
//! MaDRPC (Massively Distributed RPC) is a Rust-based distributed RPC system
//! that enables parallel computation across multiple nodes. This crate contains
//! the shared protocol and transport infrastructure used by all components:
//!
//! - **Protocol Layer**: Request/Response types, error handling, and type definitions
//! - **Transport Layer**: HTTP-based communication with JSON-RPC serialization
//!
//! # Architecture
//!
//! The system uses standard HTTP/JSON-RPC:
//! - **Transport**: HTTP/1.1
//! - **Protocol**: JSON-RPC 2.0 specification
//! - **Serialization**: JSON
//! - **Content-Type**: application/json
//!
//! # Components
//!
//! - [`protocol`] - Core protocol types (Request, Response, Error, JSON-RPC types)
//! - [`transport`] - HTTP transport implementations
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

pub mod auth;
pub mod protocol;
pub mod transport;

pub use auth::*;
pub use protocol::*;
