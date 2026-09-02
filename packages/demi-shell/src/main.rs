//! demi-shell: the QuickJS shell that runs Demi's embedded bundle.
//!
//! One binary, two entry modes selected by the bundle from `argv[0]`
//! (`docs/demi-next/shell.md`). The Rust side is the event loop, the module
//! loader and the `demishell:*` primitives; everything else is the bundle.

mod bytes;
mod embedded;
mod error;
mod event_loop;
mod fs;
mod globals;
mod handles;
mod loader;
mod net;
mod process;
mod runtime;
mod state;

fn main() {
    let code = event_loop::run();
    std::process::exit(code);
}
