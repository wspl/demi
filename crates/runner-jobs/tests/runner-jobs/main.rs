//! The integration tests of the job table, the command dispatcher and the
//! local endpoint, in one process.

mod command_client;
mod dispatch;
mod load;
mod local;
mod tasks;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here run together, and a tighter bound fails them on a
// loaded machine.
