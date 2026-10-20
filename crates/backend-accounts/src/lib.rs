//! Accounts and what a user keeps with one.
//!
//! The crate holds:
//!
//! - Accounts and password hashing.
//! - Web sessions and the login lockout.
//! - Email-change delivery.
//! - Each user's preferences.
//! - Each user's subagent settings and personal instructions.
//! - Each user's push subscriptions, with the sending of push notifications.
//!
//! The backend gives the crate the key that email codes are hashed under and
//! the VAPID key. The crate derives no keys.
//!
//! The crate knows no conversations, devices or providers.

// The text of the mail the backend sends, one catalogue per language.
rust_i18n::i18n!("locales", fallback = "en");

pub mod email_change;
pub mod instructions;
pub mod login_limiter;
pub mod passkeys;
pub mod passwords;
pub mod push;
pub mod sessions;
pub mod settings;
pub mod subagents;
pub mod user_agent;
