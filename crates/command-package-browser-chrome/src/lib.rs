//! The conversation browser's Chrome (`crates-and-packages.md`
//! § command-package-browser-chrome), in five layers, each a module that knows
//! only the ones beneath it: Chrome and the operations run on it (`driver`),
//! each environment and its tabs (`tabs`), what the commands do to a page
//! (`page`), raw CDP commands and WebMCP (`cdp`), and the live view (`live`).
//! The `demi.browser` program (`command-package-browser`) composes them.

pub mod cdp;
pub mod driver;
pub mod live;
pub mod page;
pub mod tabs;
