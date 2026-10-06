//! When Demi reclaims what a conversation uses on a Host
//! (`resource-lifecycle.md`), over the idle watch of `backend-idle-watch`: each
//! Host of an idle conversation, a paired device or a running Cloud, hears
//! the conversation release ([`conversations`]); a Cloud also stops as a
//! whole (`managed`).

pub mod conversations;
