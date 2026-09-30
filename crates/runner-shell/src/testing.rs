//! What the shell's tests reach besides its public items (feature
//! `testing`): a job's shell and its scope, where a test watches the job's
//! units wait, and the standard utilities one by one.

pub use crate::{
    interpreter::{ShellOptions, ShellResult, execute},
    job::Job,
    scope::{Activity, Scope},
    utilities::{UTILITIES, run},
};
