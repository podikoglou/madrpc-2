//! MaDRPC Transport Layer
//!
//! This module provides TCP transport and codecs for sending/receiving RPC messages.

pub mod codec;
pub mod tcp;
pub mod tcp_server;

pub use codec::{Codec, JsonCodec};
pub use tcp::{TcpTransport, TcpTransportAsync};
pub use tcp_server::TcpServer;

#[cfg(test)]
mod tests;
