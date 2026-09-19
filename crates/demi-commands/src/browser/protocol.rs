//! Browser business contracts generated from @demicodes/browser-protocol.
#[allow(
    dead_code,
    unused_variables,
    clippy::needless_borrow,
    clippy::deref_addrof,
    clippy::len_zero,
    clippy::nonminimal_bool,
    clippy::collapsible_if,
    clippy::redundant_closure_call,
    clippy::clone_on_copy,
    clippy::large_enum_variant,
    clippy::neg_cmp_op_on_partial_ord
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/browser.rs"));
}
pub(crate) use generated::*;
