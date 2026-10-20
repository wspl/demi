//! Provider assembly and model catalogs (`crates/backend-providers/src/lib.rs`,
//! `crates/shared-types/src/catalog.rs`).
//!
//! The module holds:
//!
//! - The contract of the families entries are built with.
//! - The provider of each entry and account.
//! - The catalog of each entry, with its cache.
//! - The vendors from models.dev.
//! - What the product shows of a provider.
//! - The vendor's Claude Code releases.
//! - The user's small model.

pub mod assembly;
pub mod catalog;
pub mod catalog_cache;
pub mod claude_releases;
pub mod details;
pub mod families;
pub mod rotation;
pub mod small_model;
pub mod vendors;
