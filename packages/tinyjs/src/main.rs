//! tinyjs: the QuickJS runtime that runs Demi's embedded bundle.
//!
//! A prebuilt runtime: the bundle is packed onto a copy of the binary (see
//! `payload.rs`), or, on the bare binary, named on the command line
//! (`docs/demi-next/tinyjs.md`). The Rust side is the event loop, the module
//! loader and the `tinyjs:*` primitives; everything else is the bundle.

mod bytes;
mod error;
mod event_loop;
mod fs;
mod globals;
mod handles;
mod loader;
mod net;
mod payload;
mod process;
mod runtime;
mod state;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (payload, argv) = match payload::packed() {
        Some(source) => (payload::Payload::Packed { source }, args),
        None => {
            let Some(entry) = args.get(1) else {
                eprintln!("usage: tinyjs <entry.mjs> [args...]");
                std::process::exit(2);
            };
            match payload::directory(entry) {
                Ok(p) => (p, args[1..].to_vec()),
                Err(e) => {
                    eprintln!("tinyjs: cannot open entry '{entry}': {e}");
                    std::process::exit(2);
                }
            }
        }
    };
    let code = event_loop::run(payload, argv);
    std::process::exit(code);
}
