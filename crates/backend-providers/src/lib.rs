//! Providers: the server's side of Demi's inference services.
//!
//! The crate has two parts:
//!
//! - `vault`: the credential vault with its records, their encryption and
//!   scope, subscription accounts and login flows. It is the provider
//!   registry's store (`crates/provider-registry/src/store.rs`).
//! - `usage`: usage metering.
//!
//! A provider connects Demi's agent to one inference service: the Anthropic
//! API, an OpenAI-compatible API, Gemini, a Codex or Grok Build subscription,
//! or Claude Code. The provider registry (`crates/provider-registry`) builds
//! each entry's provider and keeps its catalog and its accounts' rotation;
//! this crate owns what a server adds: the entries' sealed credentials, whose
//! entries each user infers with, logins, and metering. Each provider crate
//! owns one vendor's protocol, its token refresh and the reading of its
//! responses. A conversation selects a provider entry and a model, and never
//! holds a copy of the entry's credentials.
//!
//! HTTP inference runs in the backend. Claude Code, the one provider that needs
//! a process, runs its CLI on the Claude Code machine, whatever the
//! conversation's execution target is, and that CLI keeps its own sign-in
//! there:
//!
//! ```text
//! backend-providers          provider-registry        provider-* crates
//! +---------------------+    +-------------------+
//! | vault  entry,       |    | registry, catalog |  family  +----------+
//! |        sealed       |<---| rotation, quotas  | builds   | provider |
//! |        secret*,     |    |                   | -------> |          |
//! |        logins       |    +-------------------+          +----+-----+
//! | usage  meter        |                                        | runtime,
//! +----------+----------+                                        v one per
//!            | wraps each runtime              HTTPS to the vendor  session
//!            v                                 API; for claude-code, Host
//!     metered runtime ---------------------->  process IO to the CLI on
//!                                              the Claude Code machine
//! * none for Claude Code: the CLI keeps its sign-in on that machine
//! ```
//!
//! The crate knows no conversations, devices or Cloud.

pub mod usage;
pub mod vault;
