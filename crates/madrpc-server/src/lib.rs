//! MaDRPC Server
//!
//! This crate provides the node implementation for executing JavaScript functions
//! using the Boa JavaScript engine.

pub mod runtime;
pub mod node;

pub use runtime::MadrpcContext;
pub use node::Node;
