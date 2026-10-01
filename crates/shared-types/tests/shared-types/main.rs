//! The `core` contract at its boundary: what a stored or received value
//! decodes to and back, what is refused, the lookups the product shares with
//! the web app, and the schemas the web app's validators are generated
//! from.

mod blocks;
mod encodings;
mod lookups;
mod provider;
mod schemas;
mod selection;
