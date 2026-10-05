//! The lines a paired device's runner writes to its output, its log, that
//! the installers read to show the person at the terminal how pairing goes
//! (`runner.md` § Installation, pairing and removal). Each line starts with
//! one of these prefixes and ends with its value.

/// Before each pairing code the runner receives.
pub const PAIRING_CODE: &str = "demi-runner: pairing code: ";

/// Before the device's name, once the runner is paired.
pub const PAIRED: &str = "demi-runner: paired as ";

/// Before the command that removes the runner, the line after [`PAIRED`].
pub const REMOVAL: &str = "demi-runner: remove this runner with: ";
