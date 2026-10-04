//! The backend's end of a runner: pipe records, the connection engine with
//! the test as the runner, and a Host, jobs, callbacks and the media its
//! commands return over a real runner.

mod link;
mod media;
mod pipes;
mod runner;
