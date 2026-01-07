//! JavaScript runtime integration with Boa
//!
//! This module provides the integration between MaDRPC and the Boa JavaScript engine.
//! It handles:
//!
//! - Creating and managing Boa contexts with MaDRPC bindings
//! - Converting between JSON and JavaScript values
//! - Registering native Rust functions for JavaScript to call
//! - Managing async job execution for promises and async functions
//!
//! # Thread Safety
//!
//! The [`MadrpcContext`] is designed to be used on a single thread only. It is
//! explicitly marked as `!Send` and `!Sync` using a PhantomData marker to prevent
//! accidental cross-thread usage, which would cause undefined behavior with Boa's
//! thread-local Context.
//!
//! # JavaScript Bindings
//!
//! The following global functions are exposed to JavaScript:
//!
//! - `madrpc.register(name, function)` - Register a JavaScript function for RPC calls
//! - `madrpc.call(method, args)` - Make an async distributed RPC call (requires orchestrator)
//! - `madrpc.callSync(method, args)` - Make a synchronous blocking RPC call (requires orchestrator)

pub mod context;
pub mod job_executor;

mod bindings;
mod conversions;

#[cfg(test)]
mod tests;

pub use context::MadrpcContext;
pub use job_executor::TokioJobExecutor;
