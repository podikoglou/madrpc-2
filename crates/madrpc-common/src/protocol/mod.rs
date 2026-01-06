//! MaDRPC Protocol Definitions
//!
//! This module defines the core protocol types for MaDRPC, including requests,
//! responses, and error types used throughout the system.

pub mod error;
pub mod requests;
pub mod responses;

#[cfg(test)]
mod tests;

pub use error::{MadrpcError, Result};
pub use requests::{Request, RequestId, MethodName, RpcArgs};
pub use responses::{Response, RpcResult};
