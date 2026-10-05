//! Chrome and the operations run on it (`crates-and-packages.md`
//! § command-package-browser-chrome, `driver`): the pinned Chrome for Testing release, its
//! installation and what it needs on Linux, its launch with
//! the capture extension, the Chrome process and its profile, the operation
//! every command runs as, element handles, frames, navigation history, the
//! conversation's tab numbers, and the text and output a command answers with
//! (`browser.md` § Native driver).

use demi_command_package_browser_protocol::browser as protocol;

pub mod capture;
pub mod frames;
pub mod handles;
pub mod history;
pub mod installation;
pub mod launch;
pub mod numbers;
pub mod operation;
pub mod output;
pub mod process;
pub mod requirements;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod text;
