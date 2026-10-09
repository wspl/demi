//! A command's runtime error as GNU tools write it, `<command>: <object>:
//! <reason>` (`commands.md` § Handle an rpc call).

use std::io;

/// The reason `error` gives: an operating-system error as `strerror` words
/// it, `No such file or directory`, without Rust's ` (os error 2)`.
pub fn reason(error: &io::Error) -> String {
    let text = error.to_string();
    match error.raw_os_error() {
        Some(code) => text
            .strip_suffix(&format!(" (os error {code})"))
            .map_or_else(|| text.clone(), str::to_owned),
        None => text,
    }
}
