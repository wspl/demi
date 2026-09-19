//! The live view module (`browser-live-view.md`): viewers of the
//! conversation's browser, the capture of the tabs they watch, and their input.

pub(crate) mod capture;
mod commands;
mod frames;
mod hub;
mod input;
mod observers;
mod rate;
mod stream;
mod uploads;
mod viewer;
mod writer;

pub(super) use commands::visit;
pub(crate) use hub::Hub;
pub(crate) use observers::Observed;
pub(super) use viewer::serve;

/// The declared operation that serves a view (`native-runtime.md` § User
/// streams).
pub(crate) const OPERATION: &str = "browser.live";
