//! What the commands do to a page: element location and state, evaluation,
//! observation, queries and probes, content and screenshots, keyboard, pointer,
//! selection and select options, the actions that combine them, the clipboard,
//! uploads, downloads, fetches and assets, each a function over a tab.
//!
//! - A reference binds the browser generation, tab, frame document generation
//!   and actual node, not a selector. Navigation, reload, frame replacement and
//!   restart invalidate it; a removed or replaced node invalidates it and the
//!   old one is never resolved by finding another node with the same text; an
//!   ordinary same-document update does not. `inspect` returns a full bounded
//!   snapshot each time, with no shared "last view" cursor one agent could
//!   advance for another.
//! - A mutation needs exactly one element: one visible match among several is
//!   used, any other multiple match is `ambiguous_target`, with no implicit
//!   first match. A role-and-name locator matches accessibility-tree nodes, so
//!   what the tree leaves out (`display: none`) counts toward neither ambiguity
//!   nor `--nth`; the other locators match DOM elements whatever their
//!   rendering.
//! - Each action checks only the conditions it needs. A target not yet present
//!   is waited for and a node inserted after the snapshot is resampled, but a
//!   stored reference is not, and delivered input is never replayed. There is
//!   no implicit force and no fallback from a failed element action to a
//!   coordinate action.
//! - For a target in an embedded frame, a surface repaint is requested before
//!   the final geometry and hit checks: a stable DOM box does not prove the
//!   browser process has updated the transform that routes input to the child
//!   renderer.
//! - There is no accessibility-action command: CDP only observes the tree and
//!   has no action dispatch, so inferring actions would misreport what
//!   happened.
//! - Waits never use network-idle as completion, which breaks on pages with
//!   long-lived sockets.

use demi_command_package_browser_protocol::browser as protocol;

pub mod actions;
pub mod assets;
pub mod clipboard;
mod content;
pub mod download;
mod element;
pub mod evaluation;
pub mod fetch;
pub mod keyboard;
mod observation;
mod pointer;
pub mod probe;
mod query;
pub mod screenshot;
mod select;
mod selection;
#[cfg(feature = "testing")]
pub mod testing;
pub mod upload;
