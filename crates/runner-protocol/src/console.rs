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

/// Before why the device will not start its runner by itself after a
/// restart, which the installers show as it is (`runner.md` § Starting
/// with the computer).
pub const NO_AUTOSTART: &str =
    "demi-runner: this device will not start Demi by itself after a restart: ";

/// The line a runner the service manager did not start writes once it
/// handed the installation over to the manager's runner and ends
/// (`runner.md` § Starting with the computer); whoever follows the log goes
/// on with the manager's runner.
pub const HANDED_OVER: &str = "demi-runner: the service manager runs the runner from here on";
