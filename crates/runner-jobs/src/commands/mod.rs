//! Declared commands: execution contexts, dispatch, application callbacks,
//! artifact locations and user streams.
//!
//! ```text
//! Brush builtin -------- direct call ------> Dispatcher
//! External client ------ local forwarding -> Dispatcher
//!
//! Dispatcher ---- native operation --> command service on the same Host
//! Dispatcher ---- rpc_call ----------> backend: the node's command set
//! ```
//!
//! Both entry paths converge before the dispatcher picks the destination:
//!
//! - A builtin calls the dispatcher directly.
//! - An external client (an alias of the runner named for a root command, as
//!   `xargs` runs one) forwards raw argv, cwd, environment and the job's opaque
//!   context handle over the local endpoint.
//!
//! Either way the runner checks the context, selects the command and validates
//! its input, so a command package never receives an unvalidated command line.
//!
//! An `rpc` leaf reaches the backend as an `rpc_call` that names only its job.
//! The backend:
//!
//! 1. checks the call against its live record of the job;
//! 2. validates the arguments again;
//! 3. checks the conversation's grant of the leaf's permission category;
//! 4. dispatches in process to the invoking node's command set.
//!
//! A call without the grant ends with status 1 and reaches no handler.
//!
//! Rules that span these paths:
//!
//! - A reader that closed stdout, as `head` does, ends the call as a program
//!   writing into a closed pipe ends in a shell (141, nothing on stderr).
//! - When several causes stop a call, the first is the one reported.
//! - An `rpc` call exits only after its stdout has been drained, so the calling
//!   process has read everything.
//! - Transport loss is never a successful completion.

pub mod artifacts;
pub mod command_output;
pub mod contexts;
pub mod dispatch;
pub mod local;
pub mod rpc;
pub mod streams;
