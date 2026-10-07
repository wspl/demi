//! The tests of `demi-browser`: the built executable serving its operations,
//! conversation release, and the browser. Most browser tests start the
//! pinned Chrome for Testing and are ignored unless asked for:
//! `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features
//! demi-runner/test-fixtures --test browser -- --include-ignored`, on Linux
//! with `DEMI_TEST_CHROME_RUNTIME=<directory>` too, the pinned Chrome
//! runtime unpacked. They run
//! in parallel; the ones that read the whole process table of the test
//! process are the `browser-processes` binary.

// The service is `Send` and `Sync` deeper than the trait solver's default 128
// steps, as in the library (`src/lib.rs`).
#![recursion_limit = "256"]

mod families;
mod fixture;
mod page;
mod processes;
mod server;

mod assets;
mod cdp;
mod clipboard;
mod commands;
mod conversations;
mod download;
mod fetch;
mod fidelity;
mod launch;
mod live;
mod page_state;
mod preview;
mod repairs;
mod retirement;
mod screenshots;
mod tabs;
mod upload;
mod webmcp;
