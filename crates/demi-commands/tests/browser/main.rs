//! The browser tests, in one binary. Most start the pinned Chrome for Testing
//! and are ignored unless asked for:
//! `DEMI_TEST_CHROME=<chrome> cargo test ... --test browser -- --ignored`.

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
mod upload;
mod webmcp;
