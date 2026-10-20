//! Provider assembly and model catalogs (`crates/backend-providers/src/lib.rs`,
//! `crates/shared-types/src/catalog.rs`): the
//! contract of the families entries are built with, the provider of each
//! entry and account, the catalog of each entry with its cache, the vendors
//! from models.dev, what the product shows of a provider, the vendor's
//! Claude Code releases, and the user's small model.

pub mod assembly;
pub mod catalog;
pub mod catalog_cache;
pub mod claude_releases;
pub mod details;
pub mod families;
pub mod rotation;
pub mod small_model;
pub mod vendors;
