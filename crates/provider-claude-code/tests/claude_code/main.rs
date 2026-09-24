//! The Claude Code provider against a scripted CLI: how a run starts, keeps
//! and ends its process; the SDK MCP channel and the model's tool batches;
//! what the provider reads of the CLI's lines; and the account, its quota
//! and the catalog. No test runs the real CLI or reaches a vendor.

mod account;
mod cli;
mod mcp;
mod runs;
mod wire;
