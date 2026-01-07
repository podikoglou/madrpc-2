// Copyright 2025 MaDRPC Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! # MaDRPC CLI
//!
//! Command-line interface for the MaDRPC distributed RPC system.
//!
//! This crate provides the main entry point for running MaDRPC components:
//!
//! - **Nodes**: JavaScript execution servers that process RPC calls
//! - **Orchestrators**: Load balancers that distribute requests across nodes
//! - **Monitoring**: Real-time metrics TUI via the `top` command
//!
//! ## Architecture
//!
//! The CLI uses the `argh` crate for argument parsing and dispatches to the
//! appropriate component implementations in `madrpc-server`, `madrpc-orchestrator`,
//! and `madrpc-client`.
//!
//! ## Key Commands
//!
//! - `madrpc node`: Start a compute node with a JavaScript script
//! - `madrpc orchestrator`: Start a load balancer with health checking
//! - `madrpc top`: Monitor a server with real-time metrics
//! - `madrpc call`: Make an RPC call (outputs raw JSON for scripting)

pub mod top;
