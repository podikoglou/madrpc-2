//! MaDRPC Client
//!
//! This crate provides the RPC client for making requests to MaDRPC orchestrators.

pub mod pool;
pub mod client;

pub use pool::{ConnectionPool, PoolConfig, PooledConnection};
pub use client::MadrpcClient;
