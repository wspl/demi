//! tinyjs: the QuickJS runtime that runs Demi's embedded bundle, and
//! `tinyjsc`, the packer that puts a bundle onto a bare copy of it.
//!
//! The runtime side is the event loop, the module loader and the
//! `tinyjs:*` primitives; everything else is the bundle
//! (`docs/demi-next/tinyjs.md`). `pack` is used only by `tinyjsc`.

pub mod bytes;
pub mod error;
pub mod event_loop;
pub mod fs;
pub mod globals;
pub mod handles;
pub mod loader;
pub mod net;
pub mod pack;
pub mod payload;
pub mod process;
pub mod runtime;
pub mod state;
