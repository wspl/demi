//! What the shell's tests reach besides its public items (feature
//! `testing`): a job's shell and its scope, where a test watches the job's
//! units wait, the interpreter a script's first line names, and the standard
//! utilities one by one.

pub use brush_core::execution_host::ScriptInterpreter;
pub use crate::{
    interpreter::{ShellOptions, ShellResult, execute},
    job::Job,
    scope::{Activity, Scope},
    shebang::command as script_command,
    utilities::{UTILITIES, run},
};
