//! The integration tests of the Host operations, in one process.

mod fs;
mod git;
mod load;
mod volumes;
mod watch;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here run together, and a tighter bound fails them on a
// loaded machine.
