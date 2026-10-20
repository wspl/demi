//! When Demi reclaims what a conversation uses on a Host, by one rule: the
//! conversation's agent has been idle for the idle window. Each Host of an idle
//! conversation, a paired device or a running Cloud, hears the conversation release
//! ([`conversations`]); a Cloud also stops as a whole once no conversation using it
//! is active (`demi_backend_cloud`'s maintenance). Both run over the idle watch of
//! `backend-idle-watch`. After a conversation's deletion, its owner's blobs that
//! nothing names any more go too ([`collection`]).
//!
//! ```text
//!   gates: the agent tree's admission, the conversation's file gate,
//!          its stream gate, the Cloud's gate
//!        | read as Activity
//!        v
//!   backend-idle-watch: one watch per resource, one window
//!        | reserve, read again, retire
//!        +--> conversation watch (`conversations`) --> release on each Host
//!        +--> Cloud watch (backend-cloud)          --> stop the machine
//! ```
//!
//! Nothing a tool does on the Host, such as an open conversation browser tab or a
//! running process, postpones reclamation on its own, and no module keeps a busy
//! flag: the gates that admit work are the only source of what is activity. A
//! conversation is active while a turn of its tree runs, while a job or an admitted
//! Host operation runs (an operation the user does on what runs there, such as
//! closing a tab, is admitted and ends at once, so it restarts the window), or while
//! a user stream is open, such as the live browser view: someone watches the Host
//! and may operate it. A look at what runs there, such as listing the browser's
//! tabs, is retention, not activity, because the page makes it by itself each time
//! it is shown again and after each tool call. The page closes its view while it is
//! hidden, so a page left open keeps its conversation, and its Cloud, active only
//! while it is visible.
//!
//! The window is one setting (`tuning::LifecycleTuning`) for both consequences: new
//! activity, however brief, restarts a full window. A Cloud is idle when no
//! conversation using it, in any role, has been active within the window and no job
//! runs on it.

pub mod collection;
pub mod conversations;
