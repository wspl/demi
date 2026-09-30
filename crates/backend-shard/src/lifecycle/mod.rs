//! When Demi reclaims what a conversation uses on a Host
//! (`resource-lifecycle.md`), over the idle watch of `backend-idle`: each
//! Host of an idle conversation, a paired device or a running Cloud, hears
//! the conversation release ([`conversations`]); a Cloud also stops as a
//! whole (`managed`). Once a day, the retention pass retires expired tool
//! media and collects the blobs nothing references ([`retention`]).

pub mod conversations;
pub mod retention;
