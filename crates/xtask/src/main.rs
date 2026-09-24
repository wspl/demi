//! The repository's development commands (`crates-and-packages.md`
//! § xtask). `bun run contracts` builds the workspace and runs
//! `target/debug/xtask contracts`.

mod contracts;

use std::process::ExitCode;

const USAGE: &str = "usage: xtask contracts";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["contracts"] => match contracts::run() {
            Ok(written) => {
                for path in written {
                    println!("wrote {}", path.display());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask contracts: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
