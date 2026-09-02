//! The `tinyjs` binary: runs the packed bundle, or on a bare binary the
//! entry file named on the command line (`payload.rs`). It parses nothing
//! else; packing is `tinyjsc`'s job.

use tinyjs::{event_loop, payload};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (payload, argv) = match payload::packed() {
        Ok(Some(bytecode)) => (payload::Payload::Packed { bytecode }, args),
        Ok(None) => bare_entry(&args),
        Err(e) => {
            eprintln!("tinyjs: cannot run the packed bundle: {e}");
            std::process::exit(2);
        }
    };
    let code = event_loop::run(payload, argv);
    std::process::exit(code);
}

/// The bare binary's command line: an entry file to run.
fn bare_entry(args: &[String]) -> (payload::Payload, Vec<String>) {
    match args.get(1).map(String::as_str) {
        Some(entry) => match payload::directory(entry) {
            Ok(p) => (p, args[1..].to_vec()),
            Err(e) => {
                eprintln!("tinyjs: cannot open entry '{entry}': {e}");
                std::process::exit(2);
            }
        },
        None => {
            eprintln!("usage: tinyjs <entry.mjs> [args...]");
            std::process::exit(2);
        }
    }
}
