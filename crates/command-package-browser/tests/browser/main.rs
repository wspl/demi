//! The tests of `demi-browser`, in one binary: the built executable serving
//! its operations, conversation release, and the browser. Most browser
//! tests start the pinned Chrome for Testing and are ignored unless asked for.
//! They check the whole process table and the directory of browser profiles,
//! so they run one at a time:
//! `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features
//! demi-runner/test-fixtures --test browser -- --include-ignored
//! --test-threads=1`.

// The service is `Send` and `Sync` deeper than the trait solver's default 128
// steps, as in the library (`src/lib.rs`).
#![recursion_limit = "256"]

mod families;
mod fixture;
mod page;
mod server;

mod assets;
mod cdp;
mod clipboard;
mod commands;
mod conversations;
mod download;
mod fetch;
mod fidelity;
mod live;
mod repairs;
mod retirement;
mod tabs;
mod upload;
mod webmcp;
