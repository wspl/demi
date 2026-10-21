//! The link over memory: its lanes, their order, the window of a stream,
//! its liveness, its end and the bounds on a peer that breaks it.

mod bounds;
mod ends;
mod lanes;
mod liveness;

use std::time::Duration;

use demi_shared_link::{Incoming, Liveness};
use demi_runner_protocol::wire::{self, Frame, Inbound};
use futures_util::StreamExt as _;

/// How long a test waits for what must happen, which only guards against a
/// hang.
const HANG_GUARD: Duration = Duration::from_secs(30);

/// The liveness the tests give a link: a probe every 200 ms, lost after 500 ms
/// of silence.
const PING: Liveness = Liveness {
    interval: Duration::from_millis(200),
    timeout: Duration::from_millis(500),
};

/// A job's stop, on the job's lane.
fn kill(job: &str) -> Frame {
    wire::encode(&Inbound::JobKill {
        job_id: job.into(),
        signal: None,
    })
    .unwrap()
}

/// `bytes` of a job's input, on the job's lane.
fn input(job: &str, bytes: usize) -> Frame {
    wire::encode(&Inbound::JobStdin {
        job_id: job.into(),
        bytes: wire::WireBytes(vec![7; bytes]),
    })
    .unwrap()
}

/// A request that its reply answers, on its own lane.
fn request(id: &str) -> Frame {
    wire::encode(&Inbound::Sync { id: id.into() }).unwrap()
}

/// What the next message is.
async fn next(incoming: &mut Incoming) -> Inbound {
    let bytes = tokio::time::timeout(HANG_GUARD, incoming.next())
        .await
        .expect("a message within the hang guard")
        .expect("the link is open")
        .expect("the link has not failed");
    wire::decode(&bytes).unwrap()
}

/// A few bytes of output of the job, at `offset`.
fn output(job: &str, offset: u64) -> Frame {
    wire::encode(&wire::Outbound::JobOutput {
        job_id: job.into(),
        stream: wire::OutputStream::Stdout,
        offset,
        bytes: wire::WireBytes(vec![b'y'; 50]),
    })
    .unwrap()
}

/// `bytes` of a job's output, on the job's lane.
fn wire_output(bytes: usize) -> Frame {
    wire::encode(&wire::Outbound::JobOutput {
        job_id: "j".into(),
        stream: wire::OutputStream::Stdout,
        offset: 0,
        bytes: wire::WireBytes(vec![b'z'; bytes]),
    })
    .unwrap()
}

/// The exit of the job, the last message of its stream.
fn exit(job: &str) -> Frame {
    wire::encode(&wire::Outbound::JobExit {
        job_id: job.into(),
        exit_code: Some(0),
        signal: None,
        spawn_error: None,
        output: None,
        files: Vec::new(),
        path_changes: Vec::new(),
        files_truncated: false,
    })
    .unwrap()
}
