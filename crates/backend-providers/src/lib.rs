//! Providers: the crate behind Demi's inference services.
//!
//! The crate has three parts:
//!
//! - `llm`: provider assembly, model catalogs with the catalog cache, and the
//!   family contract every provider family implements.
//! - `vault`: the credential vault with its records, their encryption and
//!   scope, subscription accounts, login flows and quotas.
//! - `usage`: usage metering with the request rate limit.
//!
//! A provider that needs a process gets it from the user's shard, which places
//! it on the Claude Code machine.
//!
//! A provider connects Demi's agent to one inference service: the Anthropic
//! API, an OpenAI-compatible API, Gemini, a Codex or Grok Build subscription,
//! or Claude Code. This crate owns provider configuration, credentials, model
//! discovery and inference admission. Each provider crate owns one vendor's
//! protocol, its token refresh and the reading of its responses. A conversation
//! selects a provider entry and a model, and never holds a copy of the entry's
//! credentials.
//!
//! HTTP inference runs in the backend. Claude Code, the one provider that needs
//! a process, runs its CLI on the Claude Code machine, whatever the
//! conversation's execution target is, and that CLI keeps its own sign-in
//! there:
//!
//! ```text
//! backend-providers                          provider-* crates
//! +--------------------------------+
//! | vault   entry, active account, |  family   +----------+
//! |         sealed secret*, logins | builds    | provider |  one per entry
//! | llm     assembly, catalogs,    | --------> |          |  and account,
//! |         rotation               |           +----+-----+  shared
//! | usage   meter, rate limit      |                | runtime, one per
//! +---------------+----------------+                v session, on the shard
//!                 | wraps each runtime      HTTPS to the vendor API; for
//!                 v                         claude-code, Host process IO
//!          metered runtime ---------------> to the CLI on the Claude Code
//!                                           machine, which calls the vendor
//! * none for Claude Code: the CLI keeps its sign-in on that machine
//! ```
//!
//! The crate knows no conversations, devices or Cloud.

pub mod llm;
pub mod usage;
pub mod vault;
