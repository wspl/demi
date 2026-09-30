//! The coding agent (`crates-and-packages.md` § coding-agent): the harness
//! Demi's conversations run, its system prompt, and the `demi` command root
//! its shell offers: `file` and `browser`, which run in the `demi.builtin`
//! native package, `todo`, whose handlers run in the backend over the
//! node's command storage, and the product's own groups.

mod browser;
mod demi;
mod file;
mod harness;
mod todo;

pub use demi::{DemiOptions, demi_root};
pub use harness::{CodingHarness, HostResolver};
