//! The web preview's engine (`crates-and-packages.md`
//! § command-package-browser-preview, `docs/browser/preview.md` § The preview
//! engine). So far it holds the upstream requests alone (`upstream`): the
//! client with a browser's fingerprint and the local network rule on the
//! address it connects to. The stream handler that `command-package-browser`
//! composes comes with the engine; until then the upstream client is the
//! crate's boundary.

mod upstream;

pub use upstream::{Space, Upstream, UpstreamError};
