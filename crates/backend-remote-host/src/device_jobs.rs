//! A device's jobs, which outlive each connection of its runner (`crates/runner/src/connection/kept.rs`, `sessions-and-targets.md` § Recovery and
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

use demi_runner_protocol::wire::{self, Inbound, JOB_VIEW_BYTES, KeptJob};
use demi_shared_types::StreamKind;
use tokio::{sync::watch, time::Instant};
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
/// received, when the backend cannot send it again: it sent it again once
/// already, or a restarted backend knows it only from its records.
pub const NEVER_RECEIVED: &str = "its Host's runner never received it";

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
    /// Cancelled when the job's command is stopped, which stops each of its
    /// rpc calls at once, the token each handler watches being its child.
    pub(crate) stop: CancellationToken,
    /// Holds the Host's admission while the job runs.
    pub(crate) _lease: Option<GateLease>,
    /// The connection it runs on now; none while it waits for the next.
    attached: Option<WeakLink>,
    /// Since when it has waited for a connection, as far as the backend
    /// knows: since its connection ended, or since a restarted backend
    /// took it up before its runner connected.
    away_since: Option<Instant>,
    /// Changes each time the job goes on over a new connection.
    attachments: watch::Sender<u64>,
    /// How much of each stream's first `JOB_VIEW_BYTES` reached its
    /// consumer, so output the backend read again is not delivered twice.
    head: [u64; 2],
    /// Where the last output of each stream that reached its consumer ends.
    seen: [u64; 2],
    /// While its kept output is read after a new connection, what arrives
    /// meanwhile, which follows what the read delivers.
    resyncing: Option<Resync>,
    /// A restarted backend knows it only from its records: it stays, ended
    /// or not, until the agent that ran it takes it up.
    parked: bool,
    /// Whether an agent took it up and said whose it is; true for a job
    /// started here.
    claimed: watch::Sender<bool>,
    /// The instance of the runner its start went to, once it went.
    sent_to: Option<u64>,
    /// Its start, while the backend may send it again: until the runner
    /// lists the job, or the start went again once.
    start: Option<Inbound>,
    /// Whether its start waits to go again over the connection that
    /// serves: its controls wait for it.
    resending: bool,
}

/// What arrives of a job while its kept output is read again after a new
/// connection, which waits for the read to follow it: from a connection
/// lost before the read answered, until the next one's read
/// (`sessions-and-targets.md` § Recovery and persistence). It holds what a
/// job nobody follows sends, since the kept output holds the rest and the
/// end reads what it lacks.
#[derive(Default)]
struct Resync {
    /// The bytes within each stream's first `JOB_VIEW_BYTES`, in the order
    /// they came.
    first: Vec<JobOutput>,
    /// Each stream's newest bytes beyond them.
    newest: [Newest; 2],
    /// The job's end, which comes last.
    end: Option<JobEnd>,
}

impl Resync {
    /// Holds `chunk`: its part within its stream's first `JOB_VIEW_BYTES`
    /// as it is, and its part beyond them among the stream's newest bytes.
    fn hold(&mut self, chunk: JobOutput) {
        let view = JOB_VIEW_BYTES as u64;
        let within = usize::try_from(view.saturating_sub(chunk.offset))
            .unwrap_or(usize::MAX)
            .min(chunk.bytes.len());
        if within > 0 {
            self.first.push(JobOutput {
                stream: chunk.stream,
                offset: chunk.offset,
                bytes: chunk.bytes.slice(..within),
            });
        }
        if within < chunk.bytes.len() {
            self.newest[stream_index(chunk.stream)]
                .keep(chunk.offset + within as u64, &chunk.bytes[within..]);
        }
    }

    /// Delivers what it holds to `job`, and answers the end it held.
    fn release(self, job: &mut JobEntry) -> Option<JobEnd> {
        for chunk in self.first {
            job.deliver(chunk);
        }
        for (stream, newest) in [StreamKind::Stdout, StreamKind::Stderr]
            .into_iter()
            .zip(self.newest)
        {
            if !newest.is_empty() {
                job.deliver(JobOutput {
                    stream,
                    offset: newest.offset(),
                    bytes: bytes::Bytes::from(newest.into_bytes()),
                });
            }
        }
        self.end
    }
}

/// A stream's newest bytes beyond its first `JOB_VIEW_BYTES`, at most
/// `JOB_VIEW_BYTES` of them, and where they start in the stream.
#[derive(Default)]
pub(crate) struct Newest {
    offset: u64,
    bytes: Vec<u8>,
}

impl Newest {
    /// Adds `bytes` at `offset`: they continue or overlap the ones before,
    /// or replace them; the newest `JOB_VIEW_BYTES` stay.
    pub(crate) fn keep(&mut self, offset: u64, bytes: &[u8]) {
        let end = self.offset + self.bytes.len() as u64;
        if self.bytes.is_empty() || offset < self.offset || offset > end {
            self.bytes = bytes.to_vec();
            self.offset = offset;
        } else {
            self.bytes
                .truncate(usize::try_from(offset - self.offset).expect("within the bytes"));
            self.bytes.extend_from_slice(bytes);
        }
        let excess = self.bytes.len().saturating_sub(JOB_VIEW_BYTES);
        self.bytes.drain(..excess);
        self.offset += excess as u64;
    }

    pub(crate) fn offset(&self) -> u64 {
        self.offset
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Where a stream's facts are in a job's per-stream arrays.
fn stream_index(stream: StreamKind) -> usize {
    match stream {
        StreamKind::Stdout => 0,
        StreamKind::Stderr => 1,
    }
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
    /// The release and the instance the device's runner had when it last
    /// connected, from the device's record, which outlives a restart of the
    /// backend.
    pub last_release: Option<String>,
    pub last_instance: Option<u64>,
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
    /// Ended jobs the backend has no command for: their directories go.
    pub(crate) release: Vec<String>,
    /// Running jobs that go on, whose output the backend reads again.
    pub(crate) resync: Vec<(String, wire::OutputLengths)>,
    /// Jobs the same runner never received, whose starts go again over the
    /// new connection, with the commands each needs.
    pub(crate) resend: Vec<(String, Inbound, Option<CommandSelection>)>,
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
                stop: entry.stop,
                _lease: entry.lease,
                attached: Some(link.downgrade()),
                away_since: None,
                attachments,
                head: [0; 2],
                seen: [0; 2],
                resyncing: None,
                parked: false,
                claimed: watch::Sender::new(true),
                sent_to: None,
                start: None,
                resending: false,
            },
        );
        claimed
    }

    /// The start of the job `id` went to the runner of `instance`, which
    /// may not receive it: until that runner lists the job, the backend
    /// keeps `start` to send it again.
    pub(crate) fn sent(&self, id: &str, instance: u64, start: Inbound) {
        self.with(id, |job| {
            job.sent_to = Some(instance);
            job.start = Some(start);
        });
    }

    /// The start of the job `id` went again over `link`: the job goes on
    /// over it from now on. When it could not go, the connection ended
    /// first, and the start waits for the same runner's next connection.
    pub(crate) fn resent(&self, link: &Link, id: &str, start: Result<(), Inbound>) {
        self.with(id, |job| match start {
            Ok(()) if job.attached.as_ref().is_some_and(|attached| attached.is(link)) => {
                job.resending = false;
                job.attachments.send_modify(|count| *count += 1);
            }
            Ok(()) => job.resending = false,
            Err(start) => job.start = Some(start),
        });
    }

    /// Whether the start of the job `id` waits to go again over the
    /// connection that serves.
    pub(crate) fn resending(&self, id: &str) -> bool {
        self.with(id, |job| job.resending).unwrap_or(false)
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
            stop: entry.stop.clone(),
            _lease: None,
            attached: None,
            away_since: Some(Instant::now()),
            attachments: watch::Sender::new(0),
            head: [0; 2],
            seen: [0; 2],
            resyncing: None,
            parked: false,
            claimed: watch::Sender::new(false),
            sent_to: None,
            start: None,
            resending: false,
        });
        job.parked = false;
        job.origin = entry.origin;
        // Its calls, which each run once the job is claimed, stop with the
        // command that took it up.
        job.stop = entry.stop;
        job.claimed.send_replace(true);
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

    /// Whether an agent took the job `id` up, from now on; none for a job
    /// the device does not know.
    pub(crate) fn claimed(&self, id: &str) -> Option<watch::Receiver<bool>> {
        self.with(id, |job| job.claimed.subscribe())
    }

    /// Parks the job `id`, whose consumer let go of it while it runs: its
    /// end waits for the next consumer that takes it up.
    pub(crate) fn park(&self, id: &str) {
        if let Some(job) = self.0.borrow_mut().jobs.get_mut(id) {
            job.parked = true;
        }
    }

    /// Forgets the job `id`, whose start failed.
    pub(crate) fn forget(&self, id: &str) {
        self.0.borrow_mut().jobs.remove(id);
    }

    /// Runs `f` on the job `id` while the device knows it.
    pub(crate) fn with<T>(&self, id: &str, f: impl FnOnce(&mut JobEntry) -> T) -> Option<T> {
        self.0.borrow_mut().jobs.get_mut(id).map(f)
    }

    /// How long the job `id`, which runs, has waited for a connection; none
    /// while a connection serves it, and for a job that ended or that the
    /// device does not know.
    pub(crate) fn away(&self, id: &str) -> Option<std::time::Duration> {
        self.with(id, |job| {
            let since = job.away_since.filter(|_| job.shared.ended().is_none())?;
            Some(since.elapsed())
        })
        .flatten()
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
            Some(resync) => resync.hold(chunk),
            None => job.deliver(chunk),
        });
    }

    /// The job `id` ended with `end`: its consumer learns the end, after
    /// the output a read of its kept output in progress delivers, and it
    /// leaves the table then, unless it is parked, when it waits there for
    /// the agent that takes it up. False when the device does not know it.
    pub(crate) fn exited(&self, id: &str, end: JobEnd) -> bool {
        let mut table = self.0.borrow_mut();
        let Some(job) = table.jobs.get_mut(id) else {
            return false;
        };
        match &mut job.resyncing {
            Some(resync) => resync.end = Some(end),
            None => settle(&mut table, id, end),
        }
        true
    }

    /// `link` ended: the jobs on it wait for the next connection. A read of
    /// a job's kept output under way cannot answer any more: what waits
    /// behind it, the job's end among it, waits on for the next
    /// connection's read, and the job stays as it was when the first
    /// connection was lost.
    pub(crate) fn detach(&self, link: &Link) {
        for job in self.0.borrow_mut().jobs.values_mut() {
            if job.attached.as_ref().is_some_and(|attached| attached.is(link)) {
                job.attached = None;
                job.away_since = Some(Instant::now());
            }
        }
    }

    /// Ends every job as lost for `reason`, as when the device goes, after
    /// the output that waited for a read of its kept output; an end that
    /// waited too gives way to the loss, since its output cannot be read.
    pub fn lose_all(&self, reason: &str) {
        let jobs: Vec<JobEntry> = self.0.borrow_mut().jobs.drain().map(|(_, job)| job).collect();
        for job in jobs {
            job.lose(reason);
        }
    }

    /// Matches the jobs `hello` lists with those the device knows, for the
    /// new connection `link` (`sessions-and-targets.md` § Recovery and
    /// persistence): a job that runs goes on, one that ended waits for its
    /// exit, which follows the hello's answer, one the runner does not list
    /// although it is the runner its start went to is sent again, once,
    /// since that runner lists every job it received, and any other one the
    /// runner does not list is lost. A job the backend's records name and
    /// the table does not is parked for the agent that takes it up again.
    pub(crate) fn adopt(&self, link: &Link, hello: Hello) -> Adoption {
        let mut table = self.0.borrow_mut();
        let previous = table
            .runner
            .replace((hello.instance, hello.release.clone()));
        let last_release = previous
            .as_ref()
            .map(|(_, release)| release.clone())
            .or(hello.last_release);
        let last_instance = previous.map(|(instance, _)| instance).or(hello.last_instance);
        let lost = if last_release.is_some_and(|release| release != hello.release) {
            UPGRADED
        } else if last_instance == Some(hello.instance) {
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
                // A job the backend has no command for is stopped, and its
                // exit releases its directory; one that ended already is
                // released now.
                if kept.ended.is_none() {
                    adoption.stop.push(kept.job_id.clone());
                } else {
                    adoption.release.push(kept.job_id.clone());
                }
                continue;
            }
            let job = table
                .jobs
                .entry(kept.job_id.clone())
                .or_insert_with(JobEntry::parked);
            job.attached = Some(link.downgrade());
            job.away_since = None;
            job.attachments.send_modify(|count| *count += 1);
            // The runner has it: its start never goes again.
            job.start = None;
            job.resending = false;
            // A restarted backend, or one whose connection missed a
            // `job_outliving`, learns from the hello what keeps the job
            // running after its script.
            if kept.ended.is_none() {
                job.shared.outlived_by(kept.outliving.clone());
            }
            // A job that runs and printed what its consumer did not
            // receive has its kept output read again, and so does one
            // whose read a lost connection left waiting, ended or not.
            let missed = kept.output.stdout_bytes > job.seen[0]
                || kept.output.stderr_bytes > job.seen[1];
            if (kept.ended.is_none() && missed) || job.resyncing.is_some() {
                job.resyncing.get_or_insert_with(Resync::default);
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
            let Some(job) = table.jobs.get_mut(&id) else {
                continue;
            };
            // The runner its start went to, the same since, never received
            // it: it goes again, and goes on over this connection once it
            // went.
            if job.sent_to == Some(hello.instance)
                && let Some(start) = job.start.take()
            {
                job.attached = Some(link.downgrade());
                job.away_since = None;
                job.resending = true;
                adoption.resend.push((id, start, job.commands.clone()));
                continue;
            }
            if let Some(job) = table.jobs.remove(&id) {
                job.lose(lost);
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

    /// Delivers what a read of the job's kept output over `link` gave,
    /// `output` with each stream's length `lengths`, then the output that
    /// arrived meanwhile, then the end when it came meanwhile; with no
    /// `output`, only the latter two. A read over a connection that ended
    /// first delivers nothing: what waits, waits for the next one's read.
    pub(crate) fn resynced(
        &self,
        link: &Link,
        id: &str,
        output: Option<&demi_host_interface::WholeOutput>,
        lengths: wire::OutputLengths,
    ) {
        let mut table = self.0.borrow_mut();
        let Some(job) = table.jobs.get_mut(id) else {
            return;
        };
        if !job.attached.as_ref().is_some_and(|attached| attached.is(link)) {
            return;
        }
        let Some(resync) = job.resyncing.take() else {
            return;
        };
        if let Some(output) = output {
            for chunk in kept_chunks(output, lengths) {
                job.deliver(chunk);
            }
        }
        if let Some(end) = resync.release(job) {
            settle(&mut table, id, end);
        }
    }
}

/// The job `id` of `table` ended with `end`: its consumer learns the end,
/// and it leaves the table, unless it is parked, when it waits there for
/// the agent that takes it up.
fn settle(table: &mut Table, id: &str, end: JobEnd) {
    let Some(job) = table.jobs.get_mut(id) else {
        return;
    };
    job.cancel.cancel();
    job.shared.finish(end);
    if !job.parked {
        table.jobs.remove(id);
    }
}

/// What a job starts with, or is taken up with.
pub(crate) struct NewJob {
    pub(crate) shared: Rc<Shared<JobEnd, JobOutput>>,
    pub(crate) media: Rc<RefCell<Vec<JobMedium>>>,
    pub(crate) origin: Option<Rc<JobOrigin>>,
    pub(crate) commands: Option<CommandSelection>,
    pub(crate) lease: Option<GateLease>,
    pub(crate) stop: CancellationToken,
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
            stop: CancellationToken::new(),
            _lease: None,
            attached: None,
            away_since: None,
            attachments: watch::Sender::new(0),
            head: [0; 2],
            seen: [0; 2],
            resyncing: None,
            parked: true,
            claimed: watch::Sender::new(false),
            sent_to: None,
            start: None,
            resending: false,
        }
    }

    /// Ends the job as lost for `reason`, after the output that waited for
    /// a read of its kept output.
    fn lose(mut self, reason: &str) {
        if let Some(resync) = self.resyncing.take() {
            // The loss replaces the end that waited.
            let _end = resync.release(&mut self);
        }
        self.cancel.cancel();
        self.shared.finish(JobEnd::lost(reason));
    }

    /// Passes `chunk` to the consumer: the part of a stream's first
    /// `JOB_VIEW_BYTES` that reached it already is left out.
    fn deliver(&mut self, chunk: JobOutput) {
        let index = stream_index(chunk.stream);
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
    let index = stream_index;
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
