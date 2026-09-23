//! The browser tests, in one binary. Most start the pinned Chrome for Testing
//! and are ignored unless asked for. They check the whole process table and
//! the temporary directory, so they run one at a time:
//! `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features
//! demi-runner/test-fixtures,demi-commands/testing --test browser --
//! --include-ignored --test-threads=1`.

mod families;
mod fixture;
#[cfg(feature = "testing")]
mod page;
mod server;

mod assets;
mod cdp;
mod clipboard;
mod download;
mod fetch;
mod fidelity;
mod live;
mod repairs;
mod retirement;
mod tabs;
mod upload;
mod webmcp;
