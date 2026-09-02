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
        Ok(Some(bytecode)) => (payload::Payload::Packed { bytecode }, args),
        Ok(None) => bare_entry(&args),
        Err(e) => {
            eprintln!("tinyjs: cannot read the packed bundle: {e}");
            std::process::exit(2);
        }
    };
    let code = event_loop::run(payload, argv);
    std::process::exit(code);
}

/// The bare binary's command line: an entry file to run, or `--pack`.
fn bare_entry(args: &[String]) -> (payload::Payload, Vec<String>) {
    match args.get(1).map(String::as_str) {
        Some("--pack") => {
            let usage = || -> ! {
                eprintln!("usage: tinyjs --pack <bundle.mjs> --out <file> [--bin <bare tinyjs>]");
                std::process::exit(2);
            };
            let Some(bundle) = args.get(2) else { usage() };
            let mut out = None;
            let mut bin = None;
            let mut rest = args[3..].iter();
            while let Some(flag) = rest.next() {
                match (flag.as_str(), rest.next()) {
                    ("--out", Some(v)) => out = Some(v.as_str()),
                    ("--bin", Some(v)) => bin = Some(v.as_str()),
                    _ => usage(),
                }
            }
            let Some(out) = out else { usage() };
            if let Err(e) = payload::pack(bundle, out, bin) {
                eprintln!("tinyjs: pack failed: {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Some(entry) => match payload::directory(entry) {
            Ok(p) => (p, args[1..].to_vec()),
            Err(e) => {
                eprintln!("tinyjs: cannot open entry '{entry}': {e}");
                std::process::exit(2);
            }
        },
        None => {
            eprintln!("usage: tinyjs <entry.mjs> [args...]\n       tinyjs --pack <bundle.mjs> --out <file> [--bin <bare tinyjs>]");
            std::process::exit(2);
        }
    }
}
