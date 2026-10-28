//! The provider registry: the provider of each entry and account, built by
//! the entry's family from the entry's configuration, with each entry's
//! catalog and its accounts' rotation and quotas.
//!
//! A program that infers with configured entries, such as Demi's backend,
//! keeps one registry for all of them:
//!
//! ```text
//! entry (family, label, API key or accounts)
//!     |  registry: its family builds the provider, reused while the
//!     v  entry is unchanged; rotation picks an account per agent
//! provider (provider-*)  ---> catalog: configured list, models.dev or
//!     |                       the provider's directory, cached
//!     v
//! runtime, one per session
//! ```
//!
//! The modules:
//!
//! - `registry`: [`ProviderRegistry`](registry::ProviderRegistry), the
//!   provider of each entry and account, and the makers of its media.
//! - `entry`: an entry, read and decoded.
//! - `families`: the contract every provider family implements, and the
//!   families a program registers.
//! - `vendors`: the vendors from models.dev, with each one's request
//!   requirements.
//! - `catalog` and `catalog_cache`: where an entry's catalog comes from, and
//!   the cache that keeps it.
//! - `rotation` and `quotas`: which account an agent takes, and each
//!   account's quota snapshot.
//! - `details`: what a program shows of an entry's provider.
//! - `claude_releases`: the vendor's newest Claude Code release.
//! - `small_model`: the model a small job runs on.
//! - `store`: what the program stores for the registry
//!   ([`RegistryStore`](store::RegistryStore)).
//!
//! The registry keeps no credential: an API-key entry's key is part of its
//! configuration, and a subscription's accounts are read through the store's
//! credential pool. Sealing them, and every user's and instance's rule of
//! whose entries one infers with, are the program's. The crate knows no
//! conversation, device or page.

pub mod catalog;
pub mod catalog_cache;
pub mod claude_releases;
pub mod details;
pub mod entry;
pub mod families;
pub mod quotas;
pub mod registry;
pub mod rotation;
pub mod small_model;
pub mod store;
pub mod vendors;
