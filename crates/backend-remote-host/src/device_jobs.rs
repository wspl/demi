//! A device's jobs, which outlive each connection of its runner (`runner.md`
//! § Command lifetime, `sessions-and-targets.md` § Recovery and
//! persistence). A connection that ends leaves its jobs waiting for the
//! next; the next one's hello lists the jobs its runner keeps, and each job
//! the backend knows goes on there, ends with the status the runner kept, or
//! is lost with the reason the backend can tell from what it knows of the
//! device. A job a restarted backend knows only from its records is parked
//! here until the agent that ran it takes it up again.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use demi_runner_protocol::wire::{self, JOB_VIEW_BYTES, KeptJob};
use demi_shared_types::StreamKind;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_shared_gates::GateLease;

use crate::{
    WeakLink,
    link::{JobEnd, JobMedium, JobOrigin, JobOutput, Link, Shared},
    manifest::CommandSelection,
};

/// Why a job is lost when Demi was upgraded and the runner that kept it
/// replaced itself with the new release's (`upgrades.md`).
pub const UPGRADED: &str = "Demi was upgraded and the Host's runner replaced itself";
/// Why a Cloud's job is lost when its runner started anew.
pub const CLOUD_RESTARTED: &str = "the Cloud restarted";
/// Why a paired device's job is lost when its runner started anew.
pub const RUNNER_RESTARTED: &str = "the Host's runner started anew after it ended";
/// Why a job is lost that the runner which kept its connection's jobs never
/// received.
pub const NEVER_RECEIVED: &str = "its Host's runner never received it";
/// Why a job is lost that the runner stopped once its connection stayed
/// away for the grace.
pub const UNREACHED: &str =
    "the Host's runner stopped it after 10 minutes without a connection to Demi";

/// A device's jobs. Cloning it is cheap; the clones share the table.
#[derive(Clone, Default)]
pub struct DeviceJobs(Rc<RefCell<Table>>);

#[derive(Default)]
struct Table {
    jobs: HashMap<String, JobEntry>,
    /// The instance and the release of the device's runner that last said
    /// hello since the backend started.
    runner: Option<(u64, String)>,
}

/// One job of the device.
pub(crate) struct JobEntry {
    pub(crate) shared: Rc<Shared<JobEnd, JobOutput>>,
    /// The media the runner announced, in order; they outlive the entry.
    pub(crate) media: Rc<RefCell<Vec<JobMedium>>>,
    /// Whose job it is; none while a restarted backend parks it.
    pub(crate) origin: Option<Rc<JobOrigin>>,
    pub(crate) commands: Option<CommandSelection>,
    /// Cancelled when the job ends: its artifact resolutions stop.
    pub(crate) cancel: CancellationToken,
    /// Holds the Host's admission while the job runs.
    pub(crate) _lease: Option<GateLease>,
    /// The connection it runs on now; none while it waits for the next.
    attached: Option<WeakLink>,
    /// Changes each time the job goes on over a new connection.
    attachments: watch::Sender<u64>,
    /// How much of each stream's first `JOB_VIEW_BYTES` reached its
    /// consumer, so output the backend read again is not delivered twice.
    head: [u64; 2],
    /// Where the last output of each stream that reached its consumer ends.
    seen: [u64; 2],
    /// While its kept output is read after a new connection, the output
    /// that arrives meanwhile, which follows what the read delivers.
    resyncing: Option<Vec<JobOutput>>,
    /// The runner stopped it once its connection stayed away: its end is a
    /// loss, with its output kept.
    unreached: bool,
    /// A restarted backend knows it only from its records: it stays, ended
    /// or not, until the agent that ran it takes it up.
    parked: bool,
}

/// What a job's consumer holds of it.
pub(crate) struct Claimed {
    pub(crate) shared: Rc<Shared<JobEnd, JobOutput>>,
    pub(crate) media: Rc<RefCell<Vec<JobMedium>>>,
    pub(crate) attachments: watch::Receiver<u64>,
}

/// What a hello says of the runner and its jobs, with what the backend
/// knows of the device besides.
pub struct Hello {
    pub instance: u64,
    pub release: String,
    pub jobs: Vec<KeptJob>,
    /// The release the device's runner had when it last connected, from
    /// the device's record, which outlives a restart of the backend.
    pub last_release: Option<String>,
    /// Whether the device is a Cloud.
    pub managed: bool,
    /// The jobs the backend's records name for the device, which a
    /// restarted backend takes up again.
    pub recorded: HashSet<String>,
}

/// What a new connection does about the jobs its hello lists.
#[derive(Default)]
pub(crate) struct Adoption {
    /// Running jobs the backend has no command for: they are stopped.
    pub(crate) stop: Vec<String>,
    /// Running jobs that go on, whose output the backend reads again.
    pub(crate) resync: Vec<(String, wire::OutputLengths)>,
}

impl DeviceJobs {
    /// Registers a job about to start on `link`.
    pub(crate) fn start(&self, id: String, entry: NewJob, link: &Link) -> Claimed {
        let (attachments, attached) = watch::channel(0);
        let claimed = Claimed {
            shared: entry.shared.clone(),
            media: entry.media.clone(),
            attachments: attached,
        };
        self.0.borrow_mut().jobs.insert(
            id,
            JobEntry {
                shared: entry.shared,
                media: entry.media,
                origin: entry.origin,
                commands: entry.commands,
                cancel: CancellationToken::new(),
                _lease: entry.lease,
                attached: Some(link.downgrade()),
                attachments,
                head: [0; 2],
                seen: [0; 2],
                resyncing: None,
                unreached: false,
                parked: false,
            },
        );
        claimed
    }

    /// Takes up the job `id`, which the backend recorded as running: the
    /// job a new connection's hello parked, or one that waits for the
    /// device's runner, which its next hello settles.
    pub(crate) fn claim(&self, id: &str, entry: NewJob) -> Claimed {
        let mut table = self.0.borrow_mut();
        let job = table.jobs.entry(id.to_owned()).or_insert_with(|| JobEntry {
            shared: entry.shared.clone(),
            media: entry.media.clone(),
            origin: None,
            commands: None,
            cancel: CancellationToken::new(),
            _lease: None,
            attached: None,
            attachments: watch::Sender::new(0),
            head: [0; 2],
            seen: [0; 2],
            resyncing: None,
            unreached: false,
            parked: false,
        });
        job.parked = false;
        job.origin = entry.origin;
        job.commands = entry.commands;
        job._lease = entry.lease;
        let claimed = Claimed {
            shared: job.shared.clone(),
            media: job.media.clone(),
            attachments: job.attachments.subscribe(),
        };
        // A parked job that ended is its consumer's now.
        if job.shared.ended().is_some() {
            table.jobs.remove(id);
        }
        claimed
    }

    /// Forgets the job `id`, whose start failed.
    pub(crate) fn forget(&self, id: &str) {
        self.0.borrow_mut().jobs.remove(id);
    }

    /// Runs `f` on the job `id` while the device knows it.
    pub(crate) fn with<T>(&self, id: &str, f: impl FnOnce(&mut JobEntry) -> T) -> Option<T> {
        self.0.borrow_mut().jobs.get_mut(id).map(f)
    }

    /// The jobs running on `link`.
    pub(crate) fn running_on(&self, link: &Link) -> usize {
        self.0
            .borrow()
            .jobs
            .values()
            .filter(|job| job.attached.as_ref().is_some_and(|attached| attached.is(link)))
            .count()
    }

    /// Delivers one message of the job's output, unless the job is
    /// unknown; while its kept output is read again, it waits for the read.
    pub(crate) fn output(&self, id: &str, chunk: JobOutput) {
        self.with(id, |job| match &mut job.resyncing {
            Some(waiting) => waiting.push(chunk),
            None => job.deliver(chunk),
        });
    }

    /// The job `id` ended with `end`: its consumer learns the end, and it
    /// leaves the table, unless it is parked, when it waits there for the
    /// agent that takes it up. False when the device does not know it.
    pub(crate) fn exited(&self, id: &str, mut end: JobEnd) -> bool {
        let mut table = self.0.borrow_mut();
        let Some(job) = table.jobs.get_mut(id) else {
            return false;
        };
        if let Some(waiting) = job.resyncing.take() {
            for chunk in waiting {
                job.deliver(chunk);
            }
        }
        if job.unreached {
            end.lost = Some(UNREACHED.to_owned());
        }
        job.cancel.cancel();
        job.shared.finish(end);
        // A parked job's end waits for the agent that takes it up.
        if !job.parked {
            table.jobs.remove(id);
        }
        true
    }

    /// `link` ended: the jobs on it wait for the next connection.
    pub(crate) fn detach(&self, link: &Link) {
        for job in self.0.borrow_mut().jobs.values_mut() {
            if job.attached.as_ref().is_some_and(|attached| attached.is(link)) {
                job.attached = None;
                // A read again the ended connection cannot finish delivers
                // what waited for it; the next connection reads again.
                if let Some(waiting) = job.resyncing.take() {
                    for chunk in waiting {
                        job.deliver(chunk);
                    }
                }
            }
        }
    }

    /// Ends every job as lost for `reason`, as when the device goes.
    pub fn lose_all(&self, reason: &str) {
        let jobs: Vec<JobEntry> = self.0.borrow_mut().jobs.drain().map(|(_, job)| job).collect();
        for job in jobs {
            job.cancel.cancel();
            job.shared.finish(JobEnd::lost(reason));
        }
    }

    /// Matches the jobs `hello` lists with those the device knows, for the
    /// new connection `link` (`sessions-and-targets.md` § Recovery and
    /// persistence): a job that runs goes on, one that ended waits for its
    /// exit, which follows the hello's answer, and one the runner does not
    /// list is lost. A job the backend's records name and the table does
    /// not is parked for the agent that takes it up again.
    pub(crate) fn adopt(&self, link: &Link, hello: Hello) -> Adoption {
        let mut table = self.0.borrow_mut();
        let previous = table
            .runner
            .replace((hello.instance, hello.release.clone()));
        let last_release = previous
            .as_ref()
            .map(|(_, release)| release.clone())
            .or(hello.last_release);
        let lost = if last_release.is_some_and(|release| release != hello.release) {
            UPGRADED
        } else if previous.is_some_and(|(instance, _)| instance == hello.instance) {
            NEVER_RECEIVED
        } else if hello.managed {
            CLOUD_RESTARTED
        } else {
            RUNNER_RESTARTED
        };
        let mut adoption = Adoption::default();
        let listed: HashSet<&str> = hello.jobs.iter().map(|job| job.job_id.as_str()).collect();
        for kept in &hello.jobs {
            let known = table.jobs.contains_key(&kept.job_id);
            if !known && !hello.recorded.contains(&kept.job_id) {
                // A job the backend has no command for is stopped; its exit
                // releases its directory.
                if kept.ended.is_none() {
                    adoption.stop.push(kept.job_id.clone());
                }
                continue;
            }
            let job = table
                .jobs
                .entry(kept.job_id.clone())
                .or_insert_with(JobEntry::parked);
            job.attached = Some(link.downgrade());
            job.attachments.send_modify(|count| *count += 1);
            job.unreached = kept.ended.as_ref().is_some_and(|end| end.unreached);
            // A job that runs and printed what its consumer did not
            // receive has its kept output read again.
            let missed = kept.output.stdout_bytes > job.seen[0]
                || kept.output.stderr_bytes > job.seen[1];
            if kept.ended.is_none() && missed {
                job.resyncing.get_or_insert_with(Vec::new);
                adoption.resync.push((kept.job_id.clone(), kept.output));
            }
        }
        // A parked job that ended waits for its agent whatever the runner
        // lists.
        let unlisted: Vec<String> = table
            .jobs
            .iter()
            .filter(|(id, job)| {
                !listed.contains(id.as_str()) && !(job.parked && job.shared.ended().is_some())
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in unlisted {
            if let Some(job) = table.jobs.remove(&id) {
                job.cancel.cancel();
                job.shared.finish(JobEnd::lost(lost));
            }
        }
        // A recorded job the runner does not list was lost; its agent
        // learns it when it takes the job up.
        for id in hello.recorded {
            if !listed.contains(id.as_str()) && !table.jobs.contains_key(&id) {
                let job = JobEntry::parked();
                job.shared.finish(JobEnd::lost(lost));
                table.jobs.insert(id, job);
            }
        }
        adoption
    }

    /// Delivers what a read of the job's kept output gave after a new
    /// connection, `output` with each stream's length `lengths`, then the
    /// output that arrived meanwhile; with none, only the latter.
    pub(crate) fn resynced(
        &self,
        id: &str,
        output: Option<&demi_host_interface::WholeOutput>,
        lengths: wire::OutputLengths,
    ) {
        self.with(id, |job| {
            if let Some(output) = output {
                for chunk in kept_chunks(output, lengths) {
                    job.deliver(chunk);
                }
            }
            for chunk in job.resyncing.take().unwrap_or_default() {
                job.deliver(chunk);
            }
        });
    }
}

/// What a job starts with, or is taken up with.
pub(crate) struct NewJob {
    pub(crate) shared: Rc<Shared<JobEnd, JobOutput>>,
    pub(crate) media: Rc<RefCell<Vec<JobMedium>>>,
    pub(crate) origin: Option<Rc<JobOrigin>>,
    pub(crate) commands: Option<CommandSelection>,
    pub(crate) lease: Option<GateLease>,
}

impl JobEntry {
    /// A job a restarted backend knows only from its records, until its
    /// agent takes it up.
    fn parked() -> Self {
        Self {
            shared: Shared::new(),
            media: Rc::default(),
            origin: None,
            commands: None,
            cancel: CancellationToken::new(),
            _lease: None,
            attached: None,
            attachments: watch::Sender::new(0),
            head: [0; 2],
            seen: [0; 2],
            resyncing: None,
            unreached: false,
            parked: true,
        }
    }

    /// Passes `chunk` to the consumer: the part of a stream's first
    /// `JOB_VIEW_BYTES` that reached it already is left out.
    fn deliver(&mut self, chunk: JobOutput) {
        let index = match chunk.stream {
            StreamKind::Stdout => 0,
            StreamKind::Stderr => 1,
        };
        let end = chunk.offset + chunk.bytes.len() as u64;
        self.seen[index] = self.seen[index].max(end);
        if chunk.offset >= JOB_VIEW_BYTES as u64 {
            self.shared.push(chunk);
            return;
        }
        let head = self.head[index];
        if end <= head && !chunk.bytes.is_empty() {
            return;
        }
        let chunk = if chunk.offset < head {
            let skip = usize::try_from(head - chunk.offset).expect("within the chunk");
            JobOutput {
                stream: chunk.stream,
                offset: head,
                bytes: chunk.bytes.slice(skip..),
            }
        } else {
            chunk
        };
        self.head[index] = self.head[index].max(end);
        self.shared.push(chunk);
    }
}

/// The messages a read of a job's kept output stands for: each stream's
/// bytes within its first `JOB_VIEW_BYTES`, and its newest `JOB_VIEW_BYTES`
/// beyond them, at their offsets. The kept output leaves bytes out in one
/// place at most; the bytes after it end where each stream's length says.
fn kept_chunks(
    output: &demi_host_interface::WholeOutput,
    lengths: wire::OutputLengths,
) -> Vec<JobOutput> {
    use demi_host_interface::OutputRecord;
    let lengths = [lengths.stdout_bytes, lengths.stderr_bytes];
    let index = |stream: StreamKind| match stream {
        StreamKind::Stdout => 0,
        StreamKind::Stderr => 1,
    };
    let records = output.records();
    let gap = records
        .iter()
        .position(|record| matches!(record, OutputRecord::LeftOut(_)));
    // Where each record starts in its stream.
    let mut offsets: Vec<Option<u64>> = vec![None; records.len()];
    let mut before = [0u64; 2];
    for (at, record) in records.iter().enumerate().take(gap.unwrap_or(records.len())) {
        if let OutputRecord::Output(stream, bytes) = record {
            offsets[at] = Some(before[index(*stream)]);
            before[index(*stream)] += bytes.len() as u64;
        }
    }
    if let Some(gap) = gap {
        let mut after = lengths;
        for (at, record) in records.iter().enumerate().skip(gap + 1).rev() {
            if let OutputRecord::Output(stream, bytes) = record {
                let end = &mut after[index(*stream)];
                *end = end.saturating_sub(bytes.len() as u64);
                offsets[at] = Some(*end);
            }
        }
    }
    let mut chunks = Vec::new();
    let mut newest: [Vec<(u64, bytes::Bytes)>; 2] = Default::default();
    for (record, offset) in records.iter().zip(offsets) {
        let (OutputRecord::Output(stream, bytes), Some(offset)) = (record, offset) else {
            continue;
        };
        let view = JOB_VIEW_BYTES as u64;
        if offset < view {
            let take = usize::try_from((view - offset).min(bytes.len() as u64))
                .expect("within the chunk");
            chunks.push(JobOutput {
                stream: *stream,
                offset,
                bytes: bytes.slice(..take),
            });
        }
        let end = offset + bytes.len() as u64;
        if end > view {
            let start = offset.max(view);
            let skip = usize::try_from(start - offset).expect("within the chunk");
            newest[index(*stream)].push((start, bytes.slice(skip..)));
        }
    }
    for (stream, parts) in [StreamKind::Stdout, StreamKind::Stderr].into_iter().zip(newest) {
        // The stream's newest bytes, as the runner sends them while nobody
        // follows the job.
        let Some(end) = parts.last().map(|(start, bytes)| start + bytes.len() as u64) else {
            continue;
        };
        let view = JOB_VIEW_BYTES as u64;
        let from = end.saturating_sub(view).max(view);
        let mut bytes = Vec::new();
        for (start, part) in &parts {
            if start + part.len() as u64 <= from {
                continue;
            }
            let skip = usize::try_from(from.saturating_sub(*start)).expect("within the part");
            bytes.extend_from_slice(&part[skip..]);
        }
        chunks.push(JobOutput {
            stream,
            offset: from,
            bytes: bytes::Bytes::from(bytes),
        });
    }
    chunks
}
