//! MaDRPC Server
//!
//! This crate provides the node implementation for executing JavaScript functions
//! using the Boa JavaScript engine.
//!
//! # Architecture
//!
//! The server crate is responsible for:
//! - Loading and caching JavaScript script files
//! - Creating fresh Boa contexts for each request (for true parallelism)
//! - Executing JavaScript functions registered via `madrpc.register()`
//! - Handling metrics and info requests
//! - Optionally making distributed RPC calls to other nodes via an orchestrator
//!
//! # Thread Safety
//!
//! Each request creates a fresh Boa Context to enable true parallelism. This is
//! necessary because Boa's Context has thread-local state and is not thread-safe.
//! By creating a new context per request, multiple requests can execute concurrently
//! on the same node without synchronization issues.
//!
//! # Script Caching
//!
//! The node caches the script source to avoid file I/O on every request. However,
//! the script must be parsed in each request's own Context due to Boa's string
//! interner being tied to a specific Context.
//!
//! # Main Components
//!
//! - [`Node`] - Main node struct that handles RPC requests
//! - [`MadrpcContext`] - Boa context wrapper with MaDRPC bindings

pub mod http_router;
pub mod http_server;
pub mod node;
pub mod resource_limits;
pub mod runtime;

pub use http_router::NodeRouter;
pub use http_server::HttpServer;
pub use node::Node;
pub use resource_limits::ResourceLimits;
pub use runtime::MadrpcContext;
