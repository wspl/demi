//! The web preview's one rewriter (`crates-and-packages.md` § preview-rewrite):
//! addresses, JavaScript, CSS and HTML (`docs/browser/preview.md` § Rewriting).
//! The preview engine runs it natively; the page's runtime runs the same code
//! compiled to WebAssembly (`preview-rewrite-wasm`) for code the page creates.

pub mod address;
pub mod attributes;
pub mod boot;
pub mod css;
pub mod html;
pub mod javascript;
