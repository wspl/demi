//! The preview engine driven over its stream as the page's relay drives it
//! (`docs/browser/preview.md` § The stream, § The preview engine), against
//! fixture sites on loopback that the engine's test network names as public
//! sites, a local device and loopback. The spike's behavior cases that need
//! no browser: cookies and SameSite, the local network, CORS, CORP and
//! Referer, the response headers it replaces, rewriting, integrity, a
//! request sent again, WebSockets, backpressure, the fingerprint, and the
//! cookie jar the Host keeps.

mod support;

mod cookies;
mod fingerprint;
mod headers;
mod network;
mod rewriting;
mod sockets;
mod stream;
