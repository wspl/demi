// This file is part of the uutils diffutils package.
//
// For the full copyright and license information, please view the LICENSE-*
// files that was distributed with this source code.

use {uucore::context::env::ArgsOs, std::ffi::OsStr, std::ffi::OsString, std::iter::Peekable, std::path::Path, std::path::PathBuf, uucore::context::process::ExitCode};

mod cmp;
mod context_diff;
mod diff;
mod ed_diff;
mod macros;
mod normal_diff;
mod params;
mod side_diff;
mod unified_diff;
mod utils;

/// # Panics
/// Panics if the binary path cannot be determined
fn binary_path(args: &mut Peekable<ArgsOs>) -> PathBuf {
    match args.peek() {
        Some(ref s) if !s.is_empty() => PathBuf::from(s),
        _ => uucore::context::env::current_exe().unwrap(),
    }
}

/// #Panics
/// Panics if path has no UTF-8 valid name
fn name(binary_path: &Path) -> &OsStr {
    binary_path.file_stem().unwrap()
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage(name: &str) {
    uucore::context_println!("{name} {VERSION} (multi-call binary)\n");
    uucore::context_println!("Usage: {name} [function [arguments...]]\n");
    uucore::context_println!("Currently defined functions:\n");
    uucore::context_println!("    cmp, diff\n");
}

fn second_arg_error(name: &OsStr) -> ! {
    uucore::context_eprintln!("Expected utility name as second argument, got nothing.");
    usage(&name.to_string_lossy());
    uucore::context::process::exit(0);
}

fn main() -> ExitCode {
    let mut args = uucore::context::env::args_os().peekable();

    let exe_path = binary_path(&mut args);
    let exe_name = name(&exe_path);

    let util_name = if exe_name == "diffutils" {
        // Discard the item we peeked.
        let _ = args.next();

        args.peek()
            .cloned()
            .unwrap_or_else(|| second_arg_error(exe_name))
    } else {
        OsString::from(exe_name)
    };

    match util_name.to_str() {
        Some("diff") => diff::main(args),
        Some("cmp") => cmp::main(args),
        Some(name) => {
            uucore::context_eprintln!("{name}: utility not supported");
            ExitCode::from(2)
        }
        None => second_arg_error(exe_name),
    }
}
