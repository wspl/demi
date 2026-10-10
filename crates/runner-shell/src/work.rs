//! What a job runs beside its shell work, which every part of the job
//! shares: its background tasks, by the ids `$!` names them by, and the
//! process groups each part of it started, which a signal to that part
//! reaches (`runner.md` § Background tasks and timeouts, § Cancellation and
//! completion).

use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{Arc, Mutex, RwLock, RwLockReadGuard},
};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// The id of a job's first background task: one more than Linux's largest
/// process ID, so no process takes it.
pub(crate) const FIRST_TASK: i32 = 4_194_305;

/// How many ended processes' statuses a job keeps for `wait`, as bash keeps
/// those of a bounded number of its children.
const ENDED: usize = 1024;

/// One part of a job that a signal stops on its own: the job itself, a
/// background task, or the command a `timeout` runs. A part's parts are
/// within it, so a signal to a task reaches the tasks it started too.
pub(crate) struct Stop {
    #[cfg_attr(windows, expect(dead_code, reason = "Windows has no process groups to signal"))]
    parent: Option<Arc<Stop>>,
    /// Ends the part's shell work: its builtins, utilities and interpreter
    /// steps.
    pub(crate) shell: CancellationToken,
    /// Kills the part's processes.
    pub(crate) kill: CancellationToken,
    /// The last of the signals that stopped the part, if any did.
    signal: Mutex<Option<i32>>,
    /// Cancelled once a background task's list has run.
    pub(crate) done: CancellationToken,
}

impl Stop {
    /// The job's own part, which `cancellation` ends with everything in it.
    pub(crate) fn job(cancellation: &CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            parent: None,
            shell: cancellation.child_token(),
            kill: cancellation.child_token(),
            signal: Mutex::new(None),
            done: CancellationToken::new(),
        })
    }

    /// A part within this one whose shell work `shell` ends.
    pub(crate) fn part(self: &Arc<Self>, shell: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            parent: Some(self.clone()),
            shell,
            kill: self.kill.child_token(),
            signal: Mutex::new(None),
            done: CancellationToken::new(),
        })
    }

    /// The signal that stopped the part, the last of those that did.
    pub(crate) fn signal(&self) -> Option<i32> {
        *self.signal.lock().expect("the stop's signal is intact")
    }

    fn record(&self, signal: i32) {
        *self.signal.lock().expect("the stop's signal is intact") = Some(signal);
    }

    /// Ends the part's shell work and kills its processes.
    pub(crate) fn abort(&self) {
        self.shell.cancel();
        self.kill.cancel();
    }

    /// Whether this part is `part` or within it.
    #[cfg(unix)]
    fn within(self: &Arc<Self>, part: &Arc<Stop>) -> bool {
        let mut current = Some(self);
        while let Some(stop) = current {
            if Arc::ptr_eq(stop, part) {
                return true;
            }
            current = stop.parent.as_ref();
        }
        false
    }
}

/// The job's background tasks and process groups.
pub(crate) struct Work {
    /// Ends the job and everything it runs.
    pub(crate) cancellation: CancellationToken,
    tasks: Mutex<Tasks>,
    groups: Mutex<Groups>,
    /// Held to read while a process starts and enters the table, and to
    /// write while a signal goes to the table's groups: a process may act
    /// before its start returns, such as print what a script waits for
    /// before it signals the process.
    starting: RwLock<()>,
    /// The command lines of the background tasks that keep the job running
    /// once its script has ended; empty while the script runs
    /// (`runtime.md` § Results and previews).
    outliving: watch::Sender<Vec<String>>,
}

#[derive(Default)]
struct Tasks {
    started: i32,
    by_id: HashMap<i32, Arc<Stop>>,
}

#[derive(Default)]
struct Groups {
    /// By the ID of each group's leader, which is the group's ID.
    running: HashMap<i32, Group>,
    /// The statuses of the leaders that ended, newest last.
    ended: VecDeque<(i32, u8)>,
}

struct Group {
    #[cfg_attr(windows, expect(dead_code, reason = "Windows has no process groups to signal"))]
    stop: Arc<Stop>,
    /// Whether a signal that ends a process reached the group, after which
    /// the group's processes end in their own time.
    signalled: bool,
    /// The leader's status once it ended, for a leader the job's shell
    /// started; a program a utility started is no child of the shell's.
    status: Option<watch::Receiver<Option<u8>>>,
}

/// A process the job started, as `wait` finds it.
pub(crate) enum Process {
    Running(watch::Receiver<Option<u8>>),
    Ended(u8),
}

impl Work {
    pub(crate) fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            tasks: Mutex::default(),
            groups: Mutex::default(),
            starting: RwLock::default(),
            outliving: watch::Sender::new(Vec::new()),
        }
    }

    /// The script has ended, and `tasks`, by their command lines, still run.
    pub(crate) fn outlived_by(&self, tasks: Vec<String>) {
        self.outliving.send_if_modified(|current| {
            let changed = *current != tasks;
            *current = tasks;
            changed
        });
    }

    /// The background tasks that keep the job running once its script has
    /// ended, and each change of them.
    pub(crate) fn outliving(&self) -> watch::Receiver<Vec<String>> {
        self.outliving.subscribe()
    }

    /// Held while a process starts and enters the table.
    pub(crate) fn starting(&self) -> RwLockReadGuard<'_, ()> {
        self.starting.read().expect("the start lock is intact")
    }

    /// Numbers a new background task, within the job.
    pub(crate) fn add_task(&self, stop: Arc<Stop>) -> i32 {
        let mut tasks = self.tasks.lock().expect("the task table is intact");
        let id = FIRST_TASK + tasks.started;
        tasks.started += 1;
        tasks.by_id.insert(id, stop);
        id
    }

    /// The background task `id` names, if one of the job's does.
    pub(crate) fn task(&self, id: i32) -> Option<Arc<Stop>> {
        let tasks = self.tasks.lock().expect("the task table is intact");
        tasks.by_id.get(&id).cloned()
    }

    /// Records the process group `group`, which `stop`'s part started, until
    /// the entry drops. `waitable` says whether `wait` takes its leader: one
    /// the job's shell started.
    pub(crate) fn add_group(
        self: &Arc<Self>,
        group: i32,
        stop: Arc<Stop>,
        waitable: bool,
    ) -> GroupEntry {
        let (status, receiver) = if waitable {
            let (sender, receiver) = watch::channel(None);
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        let mut groups = self.groups.lock().expect("the group table is intact");
        // A process ID that ended may be taken again.
        groups.ended.retain(|&(pid, _)| pid != group);
        groups.running.insert(
            group,
            Group {
                stop,
                signalled: false,
                status: receiver,
            },
        );
        GroupEntry {
            work: self.clone(),
            group,
            status,
        }
    }

    /// The process `pid`, if it leads a group the job's shell started.
    pub(crate) fn process(&self, pid: i32) -> Option<Process> {
        let groups = self.groups.lock().expect("the group table is intact");
        if let Some(group) = groups.running.get(&pid) {
            return group.status.clone().map(Process::Running);
        }
        groups
            .ended
            .iter()
            .find(|&&(ended, _)| ended == pid)
            .map(|&(_, status)| Process::Ended(status))
    }

    /// Sends `signal` to `part` of the job as a `kill` sends it to a task
    /// (`runner.md` § Background tasks and timeouts): `KILL` ends its shell
    /// work and kills its processes; a signal that ends a process ends its
    /// shell work at its next step and reaches every process group it
    /// started, whose processes then end in their own time; any other only
    /// reaches the groups. `leaders` sends it to each group's leader alone,
    /// as `timeout --foreground` does.
    #[cfg(unix)]
    pub(crate) fn signal(&self, part: &Arc<Stop>, signal: i32, leaders: bool) -> io::Result<()> {
        if signal == libc::SIGKILL && !leaders {
            part.record(signal);
            part.abort();
            return Ok(());
        }
        let ends = ends_a_process(signal);
        if ends {
            part.record(signal);
            part.shell.cancel();
        }
        // After the cancellation, which ends a start that waits, and before
        // the table, which a start under way then is in.
        let _started = self.starting.write().expect("the start lock is intact");
        let mut groups = self.groups.lock().expect("the group table is intact");
        let mut failure = None;
        for (&id, group) in &mut groups.running {
            if !group.stop.within(part) {
                continue;
            }
            // A leader signalled alone leaves the rest of its group, which
            // then ends with it, as any group does.
            group.signalled |= ends && !leaders;
            let sent = if leaders {
                demi_runner_process::process::signal_process(id, signal)
            } else {
                demi_runner_process::process::signal_group(id, signal)
            };
            if let Err(error) = sent {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    /// Windows has no signals: any but 0 ends the part's shell work and
    /// kills its processes.
    #[cfg(windows)]
    pub(crate) fn signal(&self, part: &Arc<Stop>, signal: i32, _leaders: bool) -> io::Result<()> {
        if signal != 0 {
            part.record(signal);
            part.abort();
        }
        Ok(())
    }
}

/// Whether `signal`'s default action ends a process, so that it ends a
/// task's shell work as it would end bash's subshell.
#[cfg(unix)]
fn ends_a_process(signal: i32) -> bool {
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    if signal == libc::SIGINFO {
        return false;
    }
    !matches!(
        signal,
        0 | libc::SIGCHLD
            | libc::SIGCONT
            | libc::SIGSTOP
            | libc::SIGTSTP
            | libc::SIGTTIN
            | libc::SIGTTOU
            | libc::SIGURG
            | libc::SIGWINCH
    )
}

/// A process group in the job's table, until it drops.
pub(crate) struct GroupEntry {
    work: Arc<Work>,
    group: i32,
    status: Option<watch::Sender<Option<u8>>>,
}

impl GroupEntry {
    /// Records the leader's status, as the shell gives it.
    pub(crate) fn ended(&self, status: u8) {
        if let Some(sender) = &self.status {
            sender.send_replace(Some(status));
        }
    }

    /// Whether a signal that ends a process reached the group.
    pub(crate) fn signalled(&self) -> bool {
        let groups = self.work.groups.lock().expect("the group table is intact");
        groups
            .running
            .get(&self.group)
            .is_some_and(|group| group.signalled)
    }
}

impl std::fmt::Debug for GroupEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GroupEntry")
            .field("group", &self.group)
            .finish_non_exhaustive()
    }
}

impl Drop for GroupEntry {
    fn drop(&mut self) {
        let mut groups = self.work.groups.lock().expect("the group table is intact");
        groups.running.remove(&self.group);
        if let Some(status) = self.status.as_ref().and_then(|sender| *sender.borrow()) {
            if groups.ended.len() == ENDED {
                groups.ended.pop_front();
            }
            groups.ended.push_back((self.group, status));
        }
    }
}
