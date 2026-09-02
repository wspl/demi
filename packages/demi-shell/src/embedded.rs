//! The modules compiled into the binary, keyed by their `/embedded/` name.
//!
//! The production build carries the bundle; the `conformance` feature swaps
//! in the primitive conformance suite, which is the shell's definition of
//! done and runs as the embedded bundle so that it sees `demishell:*`.

pub const ENTRY: &str = "/embedded/main.mjs";

#[cfg(not(feature = "conformance"))]
pub const MODULES: &[(&str, &str)] = &[(ENTRY, include_str!("../bundle/main.mjs"))];

#[cfg(feature = "conformance")]
pub const MODULES: &[(&str, &str)] = &[
    (ENTRY, include_str!("../conformance/main.mjs")),
    ("/embedded/harness.mjs", include_str!("../conformance/harness.mjs")),
    ("/embedded/globals.mjs", include_str!("../conformance/globals.mjs")),
    ("/embedded/timers.mjs", include_str!("../conformance/timers.mjs")),
    ("/embedded/bytes.mjs", include_str!("../conformance/bytes.mjs")),
    ("/embedded/fs.mjs", include_str!("../conformance/fs.mjs")),
    ("/embedded/runtime.mjs", include_str!("../conformance/runtime.mjs")),
    ("/embedded/loader.mjs", include_str!("../conformance/loader.mjs")),
];

pub fn source(name: &str) -> Option<&'static str> {
    MODULES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}
