//! `demi-server supervise`: what systemd does for a server's services, in a
//! container, which has no systemd. It runs as the container's PID 1, from
//! the units the release carries, so a release that changes how a service
//! runs brings the change itself:
//!
//! | The units ask                      | The supervisor                     |
//! | ---------------------------------- | ---------------------------------- |
//! | `Type=notify`                      | a `NOTIFY_SOCKET`, waits for ready |
//! | `User=`, `Group=`, `UMask=`, ...   | sets them in the started process   |
//! | `RuntimeDirectory=`                | makes the directories under `/run` |
//! | `Restart=on-failure`, `RestartSec` | starts a failed service again      |
//! | `KillMode=process`                 | signals the main process alone     |
//! | `ExecStopPost=`                    | runs it once the process ended     |
//! | a `.path` unit's `PathExists=`     | starts its service while it exists |
//! | the machine's shutdown             | `SIGTERM`, `SIGINT`: stops them    |
//!
//! ```text
//!   demi-server, the manager --- requests (socket) ---+
//!   each child, and the orphans PID 1 inherits -------+--> one loop: each
//!   READY=1 (one notify socket per service) ----------+    unit's phase
//!   SIGTERM, SIGINT, SIGCHLD -------------------------+
//!   each second: restarts due, update request files --+
//! ```
//!
//! It reads a unit only while `current` names its own release. Before it
//! starts a service while `current` names another, it execs that release's
//! `demi-server supervise`, so after a move's switch the new release
//! supervises. No service runs then; what does, the update screen, the move
//! itself, the work that started it and each service's log, goes to the next
//! supervisor in [`STATE`]. A signal that arrives during the exec is lost.
//!
//! A container's processes start in the root of its cgroup hierarchy, where
//! cgroup v2 enables no controller for the cgroups beneath while processes
//! live in it, and the manager's limits enable them there. So a supervisor
//! that may write the hierarchy, as in a privileged container, first moves
//! the container's processes into a cgroup of their own, `demi-server`, as
//! Docker's docker-in-docker image does, and what it starts lives there too.
//!
//! Each service writes into a pipe the supervisor keeps all its life and
//! hands on, so a Cloud the manager left running never writes into a closed
//! one. Each line goes on to the container's output, and the last ones are
//! the service's log, which a failed move reports.

mod unit;

use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{self, BufRead as _, Write as _},
    os::{
        fd::{AsRawFd as _, BorrowedFd, FromRawFd as _, OwnedFd, RawFd},
        unix::{
            fs::PermissionsExt as _,
            net::{UnixDatagram, UnixListener},
            process::CommandExt as _,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use demi_supervisor_protocol::{Answer, Request};
use rustix::{
    io::{Errno, FdFlags},
    process::{Pid, Signal, WaitOptions, WaitStatus},
    thread::{CapabilitySet, CapabilitySets},
};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    journal::Journal,
    layout::{Layout, PATH_UNITS, UNITS},
    settings::Settings,
};
use unit::{Kind, PathUnit, Service};

/// The variable that carries what runs to the supervisor this one execs.
/// That is another release's: [`Handed`] only gains fields, which a release
/// that does not know them leaves out or skips.
const STATE: &str = "DEMI_SUPERVISOR_STATE";

/// The lines of a service's log that answer a `log` request: as many as
/// `journalctl -n 30` gives a failed move's report under systemd.
const LOG_LINES: usize = 30;

/// The `PATH` systemd gives a service.
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The work the supervisor starts before the services, which starts them
/// itself, such as a setup: a program and its arguments.
pub struct First {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// What runs, for the supervisor this one execs.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Handed {
    first: Option<i32>,
    screen: Option<i32>,
    /// The oneshot services that run, such as a requested update's.
    running: Vec<(String, i32)>,
    logs: Vec<HandedLog>,
    paths: Vec<String>,
    queue: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct HandedLog {
    unit: String,
    read: RawFd,
    write: RawFd,
    lines: Vec<String>,
}

enum Event {
    Request(Request, mpsc::Sender<Answer>),
    Ready(String),
    /// A child ended.
    Child,
    /// The container stops.
    Stop,
}

/// Where a unit is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Stopped,
    /// Its process runs, and has not reported ready yet.
    Starting(Pid),
    Running(Pid),
    /// Its process was signalled to stop.
    Stopping(Pid),
    /// Its process ended, and its stop-post command runs.
    Cleaning(Pid),
    /// It failed, and starts again then.
    Waiting(Instant),
}

/// What follows the end of a unit's process.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    /// Its way of ending decides: a failure restarts it when its unit says.
    Decide,
    Stay,
    Start,
}

struct Supervised {
    phase: Phase,
    next: Next,
    /// The definition it last started from; none for a process an earlier
    /// supervisor started.
    service: Option<Service>,
    /// Whether its process last ended in a failure.
    failed: bool,
    starting: Vec<mpsc::Sender<Answer>>,
    stopping: Vec<mpsc::Sender<Answer>>,
}

impl Supervised {
    fn new() -> Self {
        Self {
            phase: Phase::Stopped,
            next: Next::Decide,
            service: None,
            failed: false,
            starting: Vec::new(),
            stopping: Vec::new(),
        }
    }

    fn pid(&self) -> Option<Pid> {
        match self.phase {
            Phase::Starting(pid) | Phase::Running(pid) | Phase::Stopping(pid) | Phase::Cleaning(pid) => Some(pid),
            Phase::Stopped | Phase::Waiting(_) => None,
        }
    }

    fn oneshot(&self) -> bool {
        self.service.as_ref().is_none_or(|service| service.kind == Kind::Oneshot)
    }
}

/// A service's log: the pipe it writes into and its last lines.
struct Log {
    read: RawFd,
    write: OwnedFd,
    lines: Arc<Mutex<VecDeque<String>>>,
}

struct Supervisor<'a> {
    layout: &'a Layout,
    own: Version,
    events: mpsc::Sender<Event>,
    units: HashMap<String, Supervised>,
    logs: HashMap<String, Log>,
    notify: HashMap<String, PathBuf>,
    /// The path units watched, and once read, what each watches.
    paths: Vec<String>,
    watched: Option<Vec<PathUnit>>,
    screen: Option<Pid>,
    ending_screen: Vec<mpsc::Sender<Answer>>,
    first: Option<Pid>,
    /// The services to start in their order, and the answer the one started
    /// waits for.
    queue: VecDeque<String>,
    queued: Option<mpsc::Receiver<Answer>>,
    /// The services to stop before the container ends, and why it ends with
    /// a failure, if it does.
    stopping: Option<VecDeque<String>>,
    failure: Option<String>,
}

/// Supervises the services, after `first` when given, until the container
/// stops.
pub fn run(layout: &Layout, first: Option<First>) -> Result<(), Box<dyn std::error::Error>> {
    let handed = match std::env::var(STATE) {
        Ok(state) => Some(serde_json::from_str::<Handed>(&state)?),
        Err(_) => None,
    };
    // SAFETY: no other thread runs yet, so none reads the environment.
    unsafe { std::env::remove_var(STATE) };
    let directory = layout.supervisor_socket().parent().expect("the socket lies in a directory").to_owned();
    fs::create_dir_all(directory.join("notify"))?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;
    let (events, received) = mpsc::channel();
    listen(&layout.supervisor_socket(), events.clone())?;
    watch_signals(events.clone());
    let mut supervisor = Supervisor {
        layout,
        own: crate::own_version(),
        events,
        units: HashMap::new(),
        logs: HashMap::new(),
        notify: HashMap::new(),
        paths: Vec::new(),
        watched: None,
        screen: None,
        ending_screen: Vec::new(),
        first: None,
        queue: VecDeque::new(),
        queued: None,
        stopping: None,
        failure: None,
    };
    match handed {
        Some(handed) => supervisor.adopt(handed)?,
        None => {
            leave_root_cgroup();
            supervisor.paths = PATH_UNITS
                .into_iter()
                .filter(|unit| layout.units().join(unit).exists())
                .map(str::to_owned)
                .collect();
            match first {
                Some(first) => {
                    let mut command = Command::new(&first.program);
                    command.args(&first.args).env_clear().env("PATH", PATH).stdin(Stdio::null());
                    supervisor.first = Some(spawn(command, &Credentials::default(), &Confinement::none())?);
                }
                None => supervisor.queue = UNITS.into_iter().map(str::to_owned).collect(),
            }
        }
    }
    supervisor.serve(received)
}

impl Supervisor<'_> {
    fn adopt(&mut self, handed: Handed) -> io::Result<()> {
        for log in handed.logs {
            for fd in [log.read, log.write] {
                // SAFETY: the supervisor before this one left the descriptor
                // open for this one alone.
                rustix::io::fcntl_setfd(unsafe { BorrowedFd::borrow_raw(fd) }, FdFlags::CLOEXEC)?;
            }
            let lines = Arc::new(Mutex::new(log.lines.into_iter().collect()));
            // SAFETY: as above, and nothing else here owns them.
            let (read, write) = unsafe { (fs::File::from_raw_fd(log.read), OwnedFd::from_raw_fd(log.write)) };
            let fd = read.as_raw_fd();
            read_log(read, lines.clone());
            self.logs.insert(log.unit, Log { read: fd, write, lines });
        }
        for (unit, pid) in handed.running {
            let mut supervised = Supervised::new();
            supervised.phase = Phase::Running(Pid::from_raw(pid).ok_or_else(|| io::Error::other("no process id"))?);
            self.units.insert(unit, supervised);
        }
        self.first = handed.first.and_then(Pid::from_raw);
        self.screen = handed.screen.and_then(Pid::from_raw);
        self.paths = handed.paths;
        self.queue = handed.queue.into();
        Ok(())
    }

    fn serve(mut self, events: mpsc::Receiver<Event>) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            match events.recv_timeout(Duration::from_secs(1)) {
                Ok(event) => self.handle(event),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => unreachable!("the supervisor holds a sender"),
            }
            self.reap();
            self.tick();
            if self.ended() {
                return match self.failure.take() {
                    Some(failure) => Err(failure.into()),
                    None => Ok(()),
                };
            }
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Request(request, answer) => self.request(request, answer),
            Event::Ready(unit) => {
                let supervised = self.unit(&unit);
                if let Phase::Starting(pid) = supervised.phase {
                    supervised.phase = Phase::Running(pid);
                    for waiter in supervised.starting.drain(..) {
                        let _ = waiter.send(Answer::Done);
                    }
                }
            }
            Event::Child => {}
            Event::Stop => self.shut_down(None),
        }
    }

    fn request(&mut self, request: Request, answer: mpsc::Sender<Answer>) {
        match request {
            Request::Start { .. } | Request::Restart { .. } | Request::Screen { .. } if self.stopping.is_some() => {
                let _ = answer.send(Answer::Failed { error: "the container is stopping".into() });
            }
            Request::Start { unit } => self.start(&unit, Some(answer)),
            Request::Stop { unit } => self.stop(&unit, Some(answer), Next::Stay),
            Request::Restart { unit } => {
                let _ = answer.send(Answer::Done);
                match self.unit(&unit).pid() {
                    Some(_) => self.stop(&unit, None, Next::Start),
                    None => self.start(&unit, None),
                }
            }
            Request::Reload { paths } => {
                self.paths = paths;
                self.watched = None;
                let _ = answer.send(Answer::Done);
            }
            Request::Log { unit } => {
                let text = match self.logs.get(&unit) {
                    Some(log) => {
                        let lines = log.lines.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        lines.iter().map(|line| format!("{line}\n")).collect()
                    }
                    None => String::new(),
                };
                let _ = answer.send(Answer::Log { text });
            }
            Request::Screen { program, config, record } => {
                let _ = answer.send(match self.screen(&program, &config, &record) {
                    Ok(()) => Answer::Done,
                    Err(error) => Answer::Failed { error: error.to_string() },
                });
            }
            Request::EndScreen => match self.screen {
                Some(pid) => {
                    let _ = rustix::process::kill_process_group(pid, Signal::TERM);
                    self.ending_screen.push(answer);
                }
                None => {
                    let _ = answer.send(Answer::Done);
                }
            },
        }
    }

    fn unit(&mut self, unit: &str) -> &mut Supervised {
        self.units.entry(unit.to_owned()).or_insert_with(Supervised::new)
    }

    /// Starts `unit` unless it runs; `waiter` hears once it is ready or
    /// failed.
    fn start(&mut self, unit: &str, waiter: Option<mpsc::Sender<Answer>>) {
        let supervised = self.unit(unit);
        match supervised.phase {
            Phase::Running(_) => {
                if let Some(waiter) = waiter {
                    let _ = waiter.send(Answer::Done);
                }
            }
            Phase::Starting(_) => supervised.starting.extend(waiter),
            Phase::Stopping(_) | Phase::Cleaning(_) => {
                supervised.next = Next::Start;
                supervised.starting.extend(waiter);
            }
            Phase::Stopped | Phase::Waiting(_) => {
                supervised.starting.extend(waiter);
                if let Err(error) = self.launch(unit) {
                    println!("demi-server supervise: {unit} did not start: {error}");
                    let supervised = self.unit(unit);
                    supervised.phase = Phase::Stopped;
                    for waiter in supervised.starting.drain(..) {
                        let _ = waiter.send(Answer::Failed { error: format!("{unit} did not start: {error}") });
                    }
                }
            }
        }
    }

    /// Stops `unit`; `waiter` hears once its process ended and its stop-post
    /// command ran, and `next` says what follows.
    fn stop(&mut self, unit: &str, waiter: Option<mpsc::Sender<Answer>>, next: Next) {
        let supervised = self.unit(unit);
        match supervised.phase {
            Phase::Stopped | Phase::Waiting(_) => {
                supervised.phase = Phase::Stopped;
                if let Some(waiter) = waiter {
                    let _ = waiter.send(Answer::Done);
                }
                if next == Next::Start {
                    self.start(unit, None);
                }
                return;
            }
            Phase::Starting(pid) | Phase::Running(pid) => {
                let process_only = supervised.service.as_ref().is_some_and(|service| service.kill_process_only);
                let _ = if process_only {
                    rustix::process::kill_process(pid, Signal::TERM)
                } else {
                    rustix::process::kill_process_group(pid, Signal::TERM)
                };
                supervised.phase = Phase::Stopping(pid);
                for starting in supervised.starting.drain(..) {
                    let _ = starting.send(Answer::Failed { error: format!("{unit} was stopped as it started") });
                }
            }
            Phase::Stopping(_) | Phase::Cleaning(_) => {}
        }
        supervised.next = next;
        supervised.stopping.extend(waiter);
    }

    /// Requires the units of this supervisor's own release, and execs the
    /// supervisor of `current`'s when it names another and no service runs.
    fn own_units(&mut self) -> io::Result<()> {
        let current = self.layout.current_version()?;
        if current == self.own {
            return Ok(());
        }
        let running = self.units.iter().filter(|(_, supervised)| supervised.pid().is_some() && !supervised.oneshot()).map(|(unit, _)| unit.as_str()).collect::<Vec<_>>();
        if !running.is_empty() {
            return Err(io::Error::other(format!(
                "current names {current}, whose supervisor cannot take over while {} runs",
                running.join(" and ")
            )));
        }
        Err(self.hand_over())
    }

    /// Execs the supervisor of `current`'s release with what runs; returns
    /// only when the exec failed.
    fn hand_over(&mut self) -> io::Error {
        let mut handed = Handed {
            first: self.first.map(Pid::as_raw_nonzero).map(i32::from),
            screen: self.screen.map(Pid::as_raw_nonzero).map(i32::from),
            running: self
                .units
                .iter()
                .filter_map(|(unit, supervised)| supervised.pid().map(|pid| (unit.clone(), pid.as_raw_nonzero().get())))
                .collect(),
            logs: Vec::new(),
            paths: self.paths.clone(),
            queue: self.queue.iter().cloned().collect(),
        };
        for (unit, log) in &self.logs {
            for fd in [log.read, log.write.as_raw_fd()] {
                // SAFETY: the log's descriptors live as long as the supervisor.
                if let Err(error) = rustix::io::fcntl_setfd(unsafe { BorrowedFd::borrow_raw(fd) }, FdFlags::empty()) {
                    return error.into();
                }
            }
            let lines = log.lines.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            handed.logs.push(HandedLog {
                unit: unit.clone(),
                read: log.read,
                write: log.write.as_raw_fd(),
                lines: lines.iter().cloned().collect(),
            });
        }
        let state = match serde_json::to_string(&handed) {
            Ok(state) => state,
            Err(error) => return io::Error::other(error),
        };
        println!("demi-server supervise: handing over to the supervisor of the current release");
        let _ = io::stdout().flush();
        Command::new(self.layout.current().join("bin/demi-server"))
            .arg("--root")
            .arg(self.layout.root())
            .arg("supervise")
            .env(STATE, state)
            .exec()
    }

    /// Starts `unit`'s process from its installed unit.
    fn launch(&mut self, unit: &str) -> io::Result<()> {
        self.own_units()?;
        let service = unit::read_service(&self.layout.units().join(unit))?;
        let credentials = Credentials::of(service.user.as_deref(), service.group.as_deref(), &service.supplementary_groups)?;
        for name in &service.runtime_directories {
            let directory = self.layout.run_directory().join(name);
            fs::create_dir_all(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(service.runtime_directory_mode))?;
            std::os::unix::fs::chown(&directory, credentials.uid, credentials.gid)?;
        }
        let mut notify = Vec::new();
        if service.kind == Kind::Notify {
            notify.push(("NOTIFY_SOCKET".to_owned(), self.notify_socket(unit, &credentials)?.display().to_string()));
        }
        let output = self.log_output(unit)?;
        let pid = self.run_as(&service, &service.exec_start, &credentials, notify, output)?;
        let supervised = self.unit(unit);
        supervised.phase = match service.kind {
            Kind::Simple => Phase::Running(pid),
            Kind::Notify | Kind::Oneshot => Phase::Starting(pid),
        };
        supervised.next = Next::Decide;
        if service.kind == Kind::Simple {
            for waiter in supervised.starting.drain(..) {
                let _ = waiter.send(Answer::Done);
            }
        }
        supervised.service = Some(service);
        Ok(())
    }

    /// Starts `command`, the first word the program, as `service`'s unit
    /// runs its commands: as its user, with its settings, its runtime
    /// directories and `extra` variables, and confined as it says.
    fn run_as(
        &self,
        service: &Service,
        command: &[String],
        credentials: &Credentials,
        extra: Vec<(String, String)>,
        (stdout, stderr): (Stdio, Stdio),
    ) -> io::Result<Pid> {
        let mut environment = credentials.environment();
        if !service.runtime_directories.is_empty() {
            let run = self.layout.run_directory();
            let paths: Vec<String> = service.runtime_directories.iter().map(|name| run.join(name).display().to_string()).collect();
            environment.push(("RUNTIME_DIRECTORY".into(), paths.join(":")));
        }
        for (name, value) in &service.environment {
            // The unit's answer to what runs it is systemd's; here it is this.
            let value = if name == "DEMI_SERVICE_MANAGER" { "supervisor" } else { value };
            environment.push((name.clone(), value.to_owned()));
        }
        // An environment file's settings win over the unit's, as systemd
        // reads them.
        for file in &service.environment_files {
            environment.extend(Settings::read(file)?.variables().iter().cloned());
        }
        environment.extend(extra);
        let mut process = Command::new(&command[0]);
        process
            .args(&command[1..])
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        spawn(process, credentials, &Confinement::of(service)?)
    }

    /// The socket `unit` says it is ready on, made once.
    fn notify_socket(&mut self, unit: &str, credentials: &Credentials) -> io::Result<PathBuf> {
        if let Some(path) = self.notify.get(unit) {
            return Ok(path.clone());
        }
        let path = self.layout.supervisor_socket().with_file_name("notify").join(unit);
        let _ = fs::remove_file(&path);
        let socket = UnixDatagram::bind(&path)?;
        std::os::unix::fs::chown(&path, credentials.uid, credentials.gid)?;
        let events = self.events.clone();
        let name = unit.to_owned();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read) = socket.recv(&mut buffer) {
                let message = String::from_utf8_lossy(&buffer[..read]);
                if message.lines().any(|line| line == "READY=1") && events.send(Event::Ready(name.clone())).is_err() {
                    return;
                }
            }
        });
        self.notify.insert(unit.to_owned(), path.clone());
        Ok(path)
    }

    /// The standard output and error of `unit`'s next process: its log's
    /// pipe, made once.
    fn log_output(&mut self, unit: &str) -> io::Result<(Stdio, Stdio)> {
        if !self.logs.contains_key(unit) {
            let (read, write) = io::pipe()?;
            let lines = Arc::new(Mutex::new(VecDeque::new()));
            let fd = read.as_raw_fd();
            read_log(read, lines.clone());
            self.logs.insert(unit.to_owned(), Log { read: fd, write: write.into(), lines });
        }
        let write = &self.logs[unit].write;
        Ok((Stdio::from(write.try_clone()?), Stdio::from(write.try_clone()?)))
    }

    /// Starts the update screen unless it runs, as the services' transient
    /// unit under systemd does (`services::Systemd::screen`).
    fn screen(&mut self, program: &Path, config: &Path, record: &Path) -> io::Result<()> {
        if self.screen.is_some() {
            return Ok(());
        }
        let credentials = Credentials::of(Some("demi"), Some("demi"), &[])?;
        let mut environment = credentials.environment();
        environment.extend(Settings::read(config)?.variables().iter().cloned());
        let mut command = Command::new(program);
        command
            .arg("--update-screen")
            .arg(record)
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null());
        let capability = vec!["CAP_NET_BIND_SERVICE".to_owned()];
        let confinement = Confinement {
            umask: 0o022,
            private_mounts: false,
            bounding: Some(capabilities(&capability)?),
            ambient: capabilities(&capability)?,
        };
        self.screen = Some(spawn(command, &credentials, &confinement)?);
        Ok(())
    }

    /// Takes each child that ended.
    fn reap(&mut self) {
        loop {
            // Any child, which `waitpid(None)` is not: that is the
            // supervisor's process group, and each child has a session of
            // its own.
            match rustix::process::wait(WaitOptions::NOHANG) {
                Ok(Some((pid, status))) => self.child_ended(pid, status),
                Ok(None) | Err(_) => return,
            }
        }
    }

    fn child_ended(&mut self, pid: Pid, status: WaitStatus) {
        if self.first == Some(pid) {
            self.first = None;
            self.first_ended();
            return;
        }
        if self.screen == Some(pid) {
            self.screen = None;
            for waiter in self.ending_screen.drain(..) {
                let _ = waiter.send(Answer::Done);
            }
            return;
        }
        let Some((unit, phase)) = self
            .units
            .iter()
            .find(|(_, supervised)| supervised.pid() == Some(pid))
            .map(|(unit, supervised)| (unit.clone(), supervised.phase))
        else {
            // An orphan PID 1 inherited, such as a process a Cloud left.
            return;
        };
        match phase {
            Phase::Cleaning(_) => self.cleaned(&unit),
            _ => self.process_ended(&unit, pid, status),
        }
    }

    /// The unit's main process ended: its answers, then its stop-post
    /// command.
    fn process_ended(&mut self, unit: &str, pid: Pid, status: WaitStatus) {
        let supervised = self.unit(unit);
        let failed = failed(status);
        supervised.failed = failed;
        let oneshot = supervised.oneshot();
        let process_only = supervised.service.as_ref().is_some_and(|service| service.kill_process_only);
        if !process_only {
            // What the service started ends with it, as its cgroup's
            // processes do under systemd.
            let _ = rustix::process::kill_process_group(pid, Signal::KILL);
        }
        for waiter in supervised.starting.drain(..) {
            let _ = waiter.send(if oneshot && !failed {
                Answer::Done
            } else {
                Answer::Failed { error: format!("{unit} ended {}", describe(status)) }
            });
        }
        if failed {
            println!("demi-server supervise: {unit} ended {}", describe(status));
        }
        let Some(service) = supervised.service.take_if(|service| service.exec_stop_post.is_some()) else {
            self.cleaned(unit);
            return;
        };
        let post = service.exec_stop_post.clone().expect("the service has a stop-post command");
        let started = Credentials::of(service.user.as_deref(), service.group.as_deref(), &service.supplementary_groups)
            .and_then(|credentials| {
                let output = self.log_output(unit)?;
                self.run_as(&service, &post, &credentials, Vec::new(), output)
            });
        self.unit(unit).service = Some(service);
        match started {
            Ok(pid) => self.unit(unit).phase = Phase::Cleaning(pid),
            Err(error) => {
                println!("demi-server supervise: the stop-post command of {unit} did not start: {error}");
                self.cleaned(unit);
            }
        }
    }

    /// The unit's process and its stop-post command ended: its runtime
    /// directories go, then what its stop or its end asks.
    fn cleaned(&mut self, unit: &str) {
        let stopping = self.stopping.is_some();
        let run = self.layout.run_directory();
        let supervised = self.unit(unit);
        if let Some(service) = &supervised.service
            && !service.runtime_directory_preserve
        {
            for name in &service.runtime_directories {
                let _ = fs::remove_dir_all(run.join(name));
            }
        }
        for waiter in supervised.stopping.drain(..) {
            let _ = waiter.send(Answer::Done);
        }
        let restarts = supervised.service.as_ref().is_some_and(|service| service.restart_on_failure);
        let next = std::mem::replace(&mut supervised.next, Next::Decide);
        supervised.phase = match next {
            Next::Decide if supervised.failed && restarts && !stopping => {
                let delay = supervised.service.as_ref().map_or(Duration::ZERO, |service| service.restart_delay);
                Phase::Waiting(Instant::now() + delay)
            }
            Next::Start if !stopping => {
                supervised.phase = Phase::Stopped;
                self.start(unit, None);
                return;
            }
            Next::Decide | Next::Stay | Next::Start => Phase::Stopped,
        };
        for waiter in supervised.starting.drain(..) {
            let _ = waiter.send(Answer::Failed { error: format!("{unit} stopped") });
        }
    }

    /// The first work ended: the services start unless it left a move to
    /// finish, and a setup that did not finish ends the container, so the
    /// platform starts it again and the setup goes on.
    fn first_ended(&mut self) {
        if self.stopping.is_some() {
            return;
        }
        match Journal::read(&self.layout.journal()) {
            Ok(Some(journal)) => {
                println!(
                    "demi-server supervise: the move from {} to {} did not finish; the update screen says why",
                    journal.from, journal.to
                );
                return;
            }
            Ok(None) => {}
            Err(error) => println!("demi-server supervise: {error}"),
        }
        if !self.layout.current().exists() || crate::setup::marker(self.layout).exists() {
            self.shut_down(Some("Demi is not set up: the setup above stopped, and the next start of the container goes on with it".into()));
            return;
        }
        self.queue = UNITS.into_iter().map(str::to_owned).collect();
    }

    /// Stops the backend, then the manager, then what else runs, and ends.
    fn shut_down(&mut self, failure: Option<String>) {
        if self.stopping.is_some() {
            return;
        }
        println!("demi-server supervise: stopping the services");
        self.failure = failure;
        self.queue.clear();
        self.queued = None;
        self.stopping = Some(UNITS.into_iter().rev().map(str::to_owned).collect());
    }

    /// What is due: the next start or stop in order, a failed service's
    /// restart, and the services the update requests start.
    fn tick(&mut self) {
        if let Some(stopping) = &mut self.stopping {
            if let Some(unit) = stopping.front().cloned() {
                match self.unit(&unit).phase {
                    Phase::Stopped | Phase::Waiting(_) => {
                        self.unit(&unit).phase = Phase::Stopped;
                        self.stopping.as_mut().expect("the container stops").pop_front();
                    }
                    Phase::Starting(_) | Phase::Running(_) => self.stop(&unit, None, Next::Stay),
                    Phase::Stopping(_) | Phase::Cleaning(_) => {}
                }
            } else {
                // What else runs ends with the container.
                let others = self.units.values().filter_map(Supervised::pid).chain(self.first).chain(self.screen);
                for pid in others.collect::<Vec<_>>() {
                    let _ = rustix::process::kill_process_group(pid, Signal::TERM);
                }
            }
            return;
        }
        if let Some(answered) = &self.queued
            && !matches!(answered.try_recv(), Err(mpsc::TryRecvError::Empty))
        {
            self.queue.pop_front();
            self.queued = None;
        }
        if self.queued.is_none()
            && let Some(unit) = self.queue.front().cloned()
        {
            let (waiter, answered) = mpsc::channel();
            self.queued = Some(answered);
            self.start(&unit, Some(waiter));
        }
        let now = Instant::now();
        let due: Vec<String> = self
            .units
            .iter()
            .filter(|(_, supervised)| matches!(supervised.phase, Phase::Waiting(at) if at <= now))
            .map(|(unit, _)| unit.clone())
            .collect();
        for unit in due {
            self.start(&unit, None);
        }
        self.watch();
    }

    /// Starts the service of each watched path unit whose file exists.
    fn watch(&mut self) {
        if self.watched.is_none() {
            // A switch read on: the next release's supervisor reads them.
            if self.layout.current_version().ok().as_ref() != Some(&self.own) {
                return;
            }
            let units = self.layout.units();
            let read = self.paths.iter().map(|unit| unit::read_path(&units.join(unit))).collect::<io::Result<Vec<_>>>();
            match read {
                Ok(watched) => self.watched = Some(watched),
                Err(error) => {
                    println!("demi-server supervise: {error}");
                    self.watched = Some(Vec::new());
                }
            }
        }
        let starts: Vec<String> = self
            .watched
            .iter()
            .flatten()
            .filter(|path| path.exists.exists())
            .map(|path| path.unit.clone())
            .collect();
        for unit in starts {
            if self.unit(&unit).phase == Phase::Stopped {
                self.start(&unit, None);
            }
        }
    }

    /// Whether the container stops and nothing runs any more.
    fn ended(&self) -> bool {
        self.stopping.as_ref().is_some_and(VecDeque::is_empty)
            && self.first.is_none()
            && self.screen.is_none()
            && self.units.values().all(|supervised| supervised.pid().is_none())
    }
}

/// Moves the processes of the root cgroup into `demi-server`, where the
/// hierarchy is cgroup v2 and writable; elsewhere nothing changes, and the
/// manager says what its limits lack.
fn leave_root_cgroup() {
    let root = Path::new("/sys/fs/cgroup");
    let in_root = fs::read_to_string("/proc/self/cgroup").is_ok_and(|own| own.lines().any(|line| line == "0::/"));
    if !in_root || !root.join("cgroup.subtree_control").exists() {
        return;
    }
    let own = root.join("demi-server");
    if fs::create_dir_all(&own).is_err() {
        return;
    }
    let Ok(processes) = fs::read_to_string(root.join("cgroup.procs")) else {
        return;
    };
    for pid in processes.lines() {
        // A process that ended meanwhile cannot move.
        let _ = fs::write(own.join("cgroup.procs"), pid);
    }
}

/// Whether a process's end is a failure, as `Restart=on-failure` counts it:
/// an exit status other than 0, or a signal other than those of a stop.
fn failed(status: WaitStatus) -> bool {
    if let Some(code) = status.exit_status() {
        return code != 0;
    }
    let clean = [Signal::HUP, Signal::INT, Signal::TERM, Signal::PIPE].map(Signal::as_raw);
    status.terminating_signal().is_none_or(|signal| !clean.contains(&signal))
}

fn describe(status: WaitStatus) -> String {
    match (status.exit_status(), status.terminating_signal()) {
        (Some(code), _) => format!("with exit status {code}"),
        (_, Some(signal)) => format!("by signal {signal}"),
        _ => "abnormally".into(),
    }
}

/// Copies each line `source` gives to the container's output, keeping the
/// last ones in `lines`.
fn read_log(source: impl io::Read + Send + 'static, lines: Arc<Mutex<VecDeque<String>>>) {
    std::thread::spawn(move || {
        let mut reader = io::BufReader::new(source);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            if !line.ends_with(b"\n") {
                line.push(b'\n');
            }
            let _ = io::stdout().lock().write_all(&line);
            let mut lines = lines.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if lines.len() == LOG_LINES {
                lines.pop_front();
            }
            lines.push_back(String::from_utf8_lossy(&line).trim_end().to_owned());
        }
    });
}

/// Serves the socket, each connection on a thread of its own, which waits
/// for the loop's answer.
fn listen(socket: &Path, events: mpsc::Sender<Event>) -> io::Result<()> {
    let _ = fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let events = events.clone();
            std::thread::spawn(move || {
                let _ = demi_supervisor_protocol::serve(stream, |request| {
                    let (answer, answered) = mpsc::channel();
                    if events.send(Event::Request(request, answer)).is_err() {
                        return Answer::Failed { error: "the supervisor stopped".into() };
                    }
                    answered.recv().unwrap_or(Answer::Failed { error: "the supervisor dropped the request".into() })
                });
            });
        }
    });
    Ok(())
}

/// Turns the container's stop signals, and each child's end, into events.
fn watch_signals(events: mpsc::Sender<Event>) {
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_io().build() else {
            return;
        };
        runtime.block_on(async move {
            use tokio::signal::unix::{SignalKind, signal};
            let (Ok(mut terminate), Ok(mut interrupt), Ok(mut child)) =
                (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::child()))
            else {
                return;
            };
            loop {
                let event = tokio::select! {
                    _ = terminate.recv() => Event::Stop,
                    _ = interrupt.recv() => Event::Stop,
                    _ = child.recv() => Event::Child,
                };
                if events.send(event).is_err() {
                    return;
                }
            }
        });
    });
}

/// Who a process runs as.
#[derive(Default)]
struct Credentials {
    uid: Option<u32>,
    gid: Option<u32>,
    groups: Option<Vec<u32>>,
    /// The user's name, home and shell, for `USER`, `HOME` and `SHELL`.
    user: Option<(String, String, String)>,
}

impl Credentials {
    /// Looks the names up in `/etc/passwd` and `/etc/group`: a user's groups
    /// are those that list it and its own, as `initgroups` gives, and the
    /// supplementary ones.
    fn of(user: Option<&str>, group: Option<&str>, supplementary: &[String]) -> io::Result<Self> {
        if user.is_none() && group.is_none() && supplementary.is_empty() {
            return Ok(Self::default());
        }
        let passwd = fs::read_to_string("/etc/passwd")?;
        let groups = fs::read_to_string("/etc/group")?;
        let entries = |text: &str| -> Vec<Vec<String>> {
            text.lines().map(|line| line.split(':').map(str::to_owned).collect()).collect()
        };
        let (passwd, groups) = (entries(&passwd), entries(&groups));
        let group_id = |name: &str| -> io::Result<u32> {
            groups
                .iter()
                .find(|entry| entry.first().map(String::as_str) == Some(name))
                .and_then(|entry| entry.get(2)?.parse().ok())
                .ok_or_else(|| io::Error::other(format!("no group {name}")))
        };
        let name = user.unwrap_or("root");
        let entry = passwd
            .iter()
            .find(|entry| entry.first().map(String::as_str) == Some(name) && entry.len() >= 7)
            .ok_or_else(|| io::Error::other(format!("no user {name}")))?;
        let uid: u32 = entry[2].parse().map_err(|_| io::Error::other(format!("{name} has no user id")))?;
        let primary: u32 = entry[3].parse().map_err(|_| io::Error::other(format!("{name} has no group id")))?;
        let gid = match group {
            Some(group) => group_id(group)?,
            None => primary,
        };
        let mut ids = vec![primary];
        for entry in &groups {
            let lists = entry.get(3).is_some_and(|members| members.split(',').any(|member| member == name));
            if lists && let Some(id) = entry.get(2).and_then(|id| id.parse().ok()) {
                ids.push(id);
            }
        }
        for group in supplementary {
            ids.push(group_id(group)?);
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(Self {
            uid: Some(uid),
            gid: Some(gid),
            groups: Some(ids),
            user: user.map(|name| (name.to_owned(), entry[5].clone(), entry[6].clone())),
        })
    }

    /// What systemd starts a service with besides its unit's settings.
    fn environment(&self) -> Vec<(String, String)> {
        let mut environment = vec![("PATH".to_owned(), PATH.to_owned())];
        if let Some((name, home, shell)) = &self.user {
            environment.push(("USER".into(), name.clone()));
            environment.push(("LOGNAME".into(), name.clone()));
            environment.push(("HOME".into(), home.clone()));
            environment.push(("SHELL".into(), shell.clone()));
        }
        environment
    }
}

/// What a process's unit takes from it besides its user.
struct Confinement {
    umask: u32,
    private_mounts: bool,
    bounding: Option<CapabilitySet>,
    ambient: CapabilitySet,
}

impl Confinement {
    fn none() -> Self {
        Self {
            umask: 0o022,
            private_mounts: false,
            bounding: None,
            ambient: CapabilitySet::empty(),
        }
    }

    fn of(service: &Service) -> io::Result<Self> {
        Ok(Self {
            umask: service.umask,
            private_mounts: service.private_mounts,
            bounding: service.capability_bounding_set.as_deref().map(capabilities).transpose()?,
            ambient: capabilities(&service.ambient_capabilities)?,
        })
    }
}

/// The capabilities `names` names, such as `CAP_NET_BIND_SERVICE`.
fn capabilities(names: &[String]) -> io::Result<CapabilitySet> {
    names.iter().try_fold(CapabilitySet::empty(), |set, name| {
        let flag = name
            .strip_prefix("CAP_")
            .and_then(CapabilitySet::from_name)
            .ok_or_else(|| io::Error::other(format!("no capability {name}")))?;
        Ok(set | flag)
    })
}

/// Starts `command` in a session of its own, as `credentials` and
/// `confinement` say; the loop takes it once it ended.
fn spawn(mut command: Command, credentials: &Credentials, confinement: &Confinement) -> io::Result<Pid> {
    let uid = credentials.uid.map(rustix::process::Uid::from_raw);
    let gid = credentials.gid.map(rustix::process::Gid::from_raw);
    let groups: Option<Vec<rustix::process::Gid>> =
        credentials.groups.as_ref().map(|ids| ids.iter().copied().map(rustix::process::Gid::from_raw).collect());
    let umask = rustix::fs::Mode::from_raw_mode(confinement.umask);
    let (private_mounts, bounding, ambient) = (confinement.private_mounts, confinement.bounding, confinement.ambient);
    // SAFETY: the closure runs in the child before its exec, and only makes
    // system calls on what it captured.
    unsafe {
        command.pre_exec(move || {
            rustix::process::setsid()?;
            rustix::process::umask(umask);
            // A container that may not copy its mount namespace runs the
            // service in its own, and the manager says what that costs.
            if private_mounts && rustix::thread::unshare_unsafe(rustix::thread::UnshareFlags::NEWNS).is_ok() {
                use rustix::mount::MountPropagationFlags;
                rustix::mount::mount_change("/", MountPropagationFlags::PRIVATE | MountPropagationFlags::REC)?;
            }
            if let Some(bounding) = bounding {
                for capability in CapabilitySet::all().iter() {
                    if bounding.contains(capability) {
                        continue;
                    }
                    match rustix::thread::remove_capability_from_bounding_set(capability) {
                        // A capability this kernel does not have.
                        Ok(()) | Err(Errno::INVAL) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            if !ambient.is_empty() {
                rustix::thread::set_keep_capabilities(true)?;
            }
            if let Some(groups) = &groups {
                rustix::thread::set_thread_groups(groups)?;
            }
            if let Some(gid) = gid {
                rustix::thread::set_thread_gid(gid)?;
            }
            if let Some(uid) = uid {
                rustix::thread::set_thread_uid(uid)?;
            }
            if !ambient.is_empty() {
                let sets = CapabilitySets {
                    effective: ambient,
                    permitted: ambient,
                    inheritable: ambient,
                };
                rustix::thread::set_capabilities(None, sets)?;
                for capability in ambient.iter() {
                    rustix::thread::configure_capability_in_ambient_set(capability, true)?;
                }
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    Pid::from_raw(i32::try_from(child.id()).map_err(io::Error::other)?).ok_or_else(|| io::Error::other("no process id"))
}
