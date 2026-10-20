//! The Claude Code provider against a scripted CLI.
//!
//! The tests cover:
//!
//! - how a run starts, keeps and ends its process;
//! - the session a new process resumes and the blocks the entries the CLI
//!   mirrors belong to;
//! - the SDK MCP channel and the model's tool batches;
//! - what the provider reads of the CLI's lines;
//! - the account, its quota and the catalog.
//!
//! No test runs the real CLI or reaches a vendor.

mod account;
mod cli;
mod mcp;
mod prepared;
mod runs;
mod session;
mod sign_in;
mod wire;
