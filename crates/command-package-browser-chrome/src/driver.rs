//! Chrome and the operations run on it: the pinned Chrome for Testing release,
//! its installation (with the Chrome runtime on Linux), its launch with the
//! capture extension, the Chrome process and its profile, the operation every
//! command runs as, element handles, frames, navigation history, the
//! conversation's tab numbers, and the text and output a command answers with.
//!
//! Chrome for Testing is used because its downloads are versioned and never
//! update themselves, so one release can be verified and reproduced across
//! Hosts; a Host installs the pinned artifact, never a moving channel. The
//! capability is offered only where a verified artifact of that exact version
//! exists: a missing one reports it unavailable and nothing stands in. An
//! update is a release change; running environments keep their executable.

use demi_command_package_browser_protocol::browser as protocol;

pub mod capture;
pub mod frames;
pub mod glibc;
pub mod handles;
pub mod history;
pub mod installation;
pub mod launch;
pub mod numbers;
pub mod operation;
pub mod output;
pub mod process;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod text;
