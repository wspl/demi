//! Provider assembly and model catalogs (`providers.md`, `models.md`): the
//! contract of the families entries are built with, the provider of each
//! entry and account, the catalog of each entry with its cache, the vendors
//! from models.dev, what the product shows of a provider, and the vendor's
//! Claude Code releases.

pub mod assembly;
pub mod catalog;
pub mod catalog_cache;
pub mod claude_releases;
pub mod details;
pub mod families;
pub mod vendors;
