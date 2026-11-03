//! The relay: one Demi server lends a provider entry to another, which uses
//! it as a provider of its own while its conversations stay where they are.
//!
//! Only inference crosses. The borrower's session sends each request to the
//! lender, which runs it on the runtime of its own entry, with that entry's
//! rotation, device and process placement, and streams the events back; a
//! tool call comes back as an event and runs where the conversation's tools
//! run.
//!
//! ```text
//! borrower                              lender
//! +-------------------------+  WebSocket, one   +--------------------------+
//! | session                 |  per session,     | the key owner's shard    |
//! |   Borrowed runtime  ----+-- Bearer key ---->|   lend::serve            |
//! |   delta::Outbox         |                   |   delta::Inbox           |
//! |                    <----+-- events ---------+-- the entry's runtime    |
//! +-------------------------+                   +--------------------------+
//!   GET provider: name, catalog, limits  ------>
//! ```
//!
//! - `wire`: the routes, headers and messages, and the copies of the
//!   provider contract's types they carry.
//! - `delta`: a session's requests as what changed since its last.
//! - `key`: a share key as a person copies it.
//! - `lend`: the lender's end of a session; the lender's server owns its
//!   keys, its routes and the entry's runtime.
//! - `borrow`: the borrower's provider.
//!
//! What the borrower never learns: the entry's configuration, its accounts,
//! which one answers, their quotas, and a failure's words and wait. A
//! failure crosses as its code, whose message the borrower composes.
//!
//! The crate knows no server: the lender's server checks the key and hands
//! [`lend::serve`] the socket's messages, and the borrower's backend builds a
//! [`borrow::Borrowed`] from the entry's key.

pub mod borrow;
mod delta;
pub mod key;
pub mod lend;
pub mod wire;
