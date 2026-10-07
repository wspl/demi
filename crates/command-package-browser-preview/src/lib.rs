//! The web preview's engine (`crates-and-packages.md`
//! § command-package-browser-preview, `docs/browser/preview.md` § The preview
//! engine): the `preview` stream's requests and WebSockets, made upstream
//! from the Host as the logical browser would make them, with the Host's
//! cookie jar, and answered rewritten. `command-package-browser` composes
//! one engine for every conversation of the Host and serves its stream.

mod cookies;
mod engine;
mod headers;
mod policy;
mod socket;
mod sourcemap;
mod stream;
mod upstream;

#[cfg(feature = "testing")]
pub mod testing;

pub use cookies::{CookieSameSite, JarCookie};
pub use engine::{Engine, EngineError};
pub use stream::{StreamError, serve};
