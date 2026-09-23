//! The `core` contract at its boundary: what a stored or received value
//! decodes to and back, what is refused, the lookups the product shares with
//! the browser, and the schemas the browser's validators are generated from.

mod blocks;
mod encodings;
mod lookups;
mod provider;
mod schemas;
