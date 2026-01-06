//! MaDRPC Common Types and Transport
//!
//! This crate provides the core protocol definitions and TCP transport layer
//! for the MaDRPC distributed RPC system.

pub mod protocol;
pub mod transport;

pub use protocol::*;
