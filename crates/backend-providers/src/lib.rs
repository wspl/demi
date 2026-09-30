//! Providers (`providers.md`, `models.md`, `usage-and-quota.md`): provider
//! assembly and model catalogs with the catalog cache, and the family
//! contract every provider family implements (`llm`); the credential vault
//! with its records, their encryption and scope, subscription accounts,
//! login flows and quotas (`vault`); and usage metering with the request
//! rate limit (`usage`). A provider that needs a process gets it from the
//! user's shard, which places it on the user's Cloud.

pub mod llm;
pub mod usage;
pub mod vault;
