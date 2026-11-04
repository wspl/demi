//! The manager's configuration (`setup.md` § Configuration):
//!
//! - Its command line.
//! - Its own `DEMI_MANAGED_*` variables.
//! - The backend's settings it shares (`DEMI_RELEASE`,
//!   `DEMI_BACKEND_PUBLIC_URL`, `DEMI_MACHINE_MANAGER_SOCKET`,
//!   `DEMI_BACKEND_RUNNER_SOCKET`).
//!
//! All are read from the deployment's one configuration file and validated at
//! startup. An invalid or unknown setting stops the manager with an error that
//! names the variable.

use std::{
    ffi::OsString,
    net::Ipv4Addr,
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
    str::FromStr,
};

use clap::{CommandFactory, FromArgMatches, parser::ValueSource};
use demi_machine_manager_protocol::{SandboxLimits, SandboxMachine};
use demi_shared_types::CloudLimit;
use demi_machine_manager_protocol::runtime::RuntimeRelease;
use ipnet::Ipv4Net;

/// Where the manager keeps its runtime bundles, locks and namespace handle.
pub const RUNTIME_DIRECTORY: &str = "/run/demi-machine-manager";

/// The socket the manager listens on unless `DEMI_MACHINE_MANAGER_SOCKET`
/// names another, the backend's default too.
const SOCKET: &str = "/run/demi-cloud/machines.sock";

/// The backend's runner socket unless `DEMI_BACKEND_RUNNER_SOCKET` names
/// another, the backend's default too.
const RUNNER_SOCKET: &str = "/run/demi-backend/runners.sock";

/// Where the host's own resolvers are named: systemd-resolved's upstream
/// file, which a host running it has, else the classic one.
const RESOLVER_FILES: [&str; 2] = ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"];

/// The Cloud machine manager: runs users' Cloud machines as gVisor
/// sandboxes and serves the backend over a Unix socket.
#[derive(Debug, clap::Parser)]
#[command(name = "demi-machine-manager", version = demi_shared_artifacts::BUILD_VERSION)]
struct Cli {
    /// Fence and save what a stopped manager left behind, then exit (the
    /// service's stop-post command).
    #[arg(long, conflicts_with = "recover_namespace")]
    recover: bool,
    /// Recover inside a saved mount namespace; only the manager starts this.
    #[arg(long, hide = true)]
    recover_namespace: bool,
    /// Validate the configuration as a start would, then exit.
    #[arg(long, conflicts_with_all = ["recover", "recover_namespace", "import", "check_host"])]
    check_config: bool,
    /// Check that this machine can run Cloud, as a start would, changing
    /// nothing, then exit; run in a private mount namespace.
    #[arg(long, conflicts_with_all = ["recover", "recover_namespace", "import"])]
    check_host: bool,
    /// Import the release's Cloud image into the state directory beside a
    /// running manager, then exit, as an upgrade does before it stops the
    /// running one.
    #[arg(long, conflicts_with_all = ["recover", "recover_namespace"])]
    import: bool,
    /// Fetch the gVisor release this manager pins into DIRECTORY, which
    /// holds a directory for each version, print the version's directory
    /// and exit, as `demi-server runtime` does for a setup or an upgrade;
    /// it reads no other setting.
    #[arg(
        long,
        value_name = "DIRECTORY",
        value_parser = absolute,
        conflicts_with_all = ["recover", "recover_namespace", "check_config", "check_host", "import"]
    )]
    fetch_gvisor: Option<PathBuf>,
    /// The server release root, whose image/ holds the Cloud image [default:
    /// the directory above the one that holds this executable].
    #[arg(long, env = "DEMI_RELEASE", value_name = "DEMI_RELEASE", value_parser = absolute)]
    release: Option<PathBuf>,
    /// The socket the backend connects to.
    #[arg(
        long,
        env = "DEMI_MACHINE_MANAGER_SOCKET",
        value_name = "DEMI_MACHINE_MANAGER_SOCKET",
        default_value = SOCKET,
        value_parser = absolute
    )]
    socket: PathBuf,
    /// The backend's runner socket, whose directory each sandbox gets as its
    /// way to the backend; its file name is the one a Cloud's runner looks
    /// for.
    #[arg(
        long,
        env = "DEMI_BACKEND_RUNNER_SOCKET",
        value_name = "DEMI_BACKEND_RUNNER_SOCKET",
        default_value = RUNNER_SOCKET,
        value_parser = runner_socket
    )]
    runner_socket: PathBuf,
    /// The persistent state directory, on one filesystem.
    #[arg(
        long,
        env = "DEMI_MANAGED_DATA",
        value_name = "DEMI_MANAGED_DATA",
        default_value = "/opt/demi/data/cloud",
        value_parser = absolute
    )]
    data: PathBuf,
    /// The pinned runsc executable [default: the pinned version's runsc in
    /// /opt/demi/gvisor].
    #[arg(long, env = "DEMI_MANAGED_RUNSC", value_name = "DEMI_MANAGED_RUNSC", value_parser = absolute)]
    runsc: Option<PathBuf>,
    /// The proot executable, which runs Simple Isolation Clouds [default:
    /// bin/proot in the server release].
    #[arg(long, env = "DEMI_MANAGED_PROOT", value_name = "DEMI_MANAGED_PROOT", value_parser = absolute)]
    proot: Option<PathBuf>,
    /// The backend's public URL: the only backend a sandbox's runner may
    /// connect to.
    #[arg(
        long,
        env = "DEMI_BACKEND_PUBLIC_URL",
        value_name = "DEMI_BACKEND_PUBLIC_URL",
        value_parser = backend_url,
        required_unless_present = "fetch_gvisor"
    )]
    backend_url: Option<url::Url>,
    /// Whether sandboxes run under cgroup CPU, memory and PID limits.
    #[arg(
        long,
        env = "DEMI_MANAGED_LIMITS",
        value_name = "DEMI_MANAGED_LIMITS",
        default_value = "on"
    )]
    limits: Switch,
    /// The CPU budget of one sandbox, with the limits on.
    #[arg(long, env = "DEMI_MANAGED_CPUS", value_name = "DEMI_MANAGED_CPUS", default_value = "2", value_parser = decimal::<NonZeroU32>)]
    cpus: NonZeroU32,
    /// The memory limit of one sandbox, in MiB, with the limits on.
    #[arg(long, env = "DEMI_MANAGED_MEM_MIB", value_name = "DEMI_MANAGED_MEM_MIB", default_value = "2048", value_parser = decimal::<NonZeroU32>)]
    mem_mib: NonZeroU32,
    /// The memory limit of the Claude Code machine's sandbox, in MiB, with
    /// the limits on: every user's CLI processes share it
    /// (`crates/backend-cloud/src/lib.rs`).
    #[arg(long, env = "DEMI_MANAGED_CLAUDE_CODE_MEM_MIB", value_name = "DEMI_MANAGED_CLAUDE_CODE_MEM_MIB", default_value = "4096", value_parser = decimal::<NonZeroU32>)]
    claude_code_mem_mib: NonZeroU32,
    /// The IPv4 pool the sandboxes' networks come from.
    #[arg(long, env = "DEMI_MANAGED_SUBNET", value_name = "DEMI_MANAGED_SUBNET", default_value = "172.30.0.0/16", value_parser = subnet)]
    subnet: Ipv4Net,
    /// How many sandboxes may run at once; each takes four addresses.
    #[arg(long, env = "DEMI_MANAGED_SLOTS", value_name = "DEMI_MANAGED_SLOTS", default_value = "256", value_parser = slots)]
    slots: u16,
    /// The resolvers a Cloud uses, separated by commas [default: on gVisor,
    /// the host's own upstream resolvers; on Simple Isolation, which shares
    /// the host's network, the host's resolver file as it is].
    #[arg(
        long,
        env = "DEMI_MANAGED_DNS",
        value_name = "DEMI_MANAGED_DNS",
        value_delimiter = ',',
        value_parser = resolver
    )]
    dns: Vec<Ipv4Addr>,
}

/// A setting that is on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Switch {
    On,
    Off,
}

/// What the process was started to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Manage Clouds, or one step around that, with this configuration.
    Manage(Config),
    /// Fetch the pinned gVisor release into this directory, then exit; a
    /// download needs none of the configuration, which a setup has not
    /// written yet when it fetches.
    FetchGvisor(PathBuf),
}

/// What the manager was started to do with its configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Recover, then serve the backend until SIGTERM (the service).
    Serve,
    /// Fence and save a stopped manager's devices, then exit.
    Recover,
    /// The same inside a saved mount namespace, as the manager's own child.
    RecoverNamespace,
    /// Validate the configuration, then exit.
    CheckConfig,
    /// Check the machine, then exit.
    CheckHost,
    /// Import the configured release's image, then exit.
    Import,
}

/// The validated configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub mode: Mode,
    pub socket: PathBuf,
    pub data: PathBuf,
    pub runsc: PathBuf,
    pub proot: PathBuf,
    pub image: PathBuf,
    pub backend_url: url::Url,
    /// The backend's runner socket (`crates/backend-http/src/lib.rs`).
    pub runner_socket: PathBuf,
    /// Each sandbox's cgroup limits; `None` with `DEMI_MANAGED_LIMITS=off`,
    /// which runs sandboxes without cgroups.
    pub limits: Option<Limits>,
    pub subnet: Ipv4Net,
    pub slots: u16,
    /// The resolvers `DEMI_MANAGED_DNS` names, if it names any.
    pub dns: Option<Vec<Ipv4Addr>>,
}

/// One sandbox's cgroup limits.
///
/// They are a capability the operator turns on or off with one setting,
/// `DEMI_MANAGED_LIMITS`: the choice belongs to the execution host, not to the
/// kind of deployment, so a production host without the cgroup controllers
/// runs with them off and a development VM that has them runs with them on.
/// Only the setting decides, and the manager never falls back from one mode to
/// the other.
///
/// With them on:
///
/// - Each sandbox runs in a cgroup of its own, limited to the CPU budget (a
///   scheduling budget, not a virtual CPU allocation), the memory limit with no
///   swap, which the sandbox sees as its total memory, and a host `pids.max`,
///   covering the Sentry and the Gofer too.
/// - The temporary mounts count toward memory.
///
/// With them off:
///
/// - The manager creates, changes and reads no cgroup, and every `runsc`
///   command adds `--ignore-cgroups`, without which `runsc` would create one
///   for each sandbox.
/// - The sandbox shares the host's CPUs and memory without a bound and sees the
///   host's memory as its total.
/// - A CPU budget or memory limit configured with them off is an error, since
///   nothing would apply it.
///
/// The log says which mode runs (a warning at startup with them off, and the
/// readiness line in both). Nothing else reports it, and the backend does not
/// learn it.
///
/// In both modes the sandbox process has `RLIMIT_NPROC` and `RLIMIT_NOFILE`
/// limits that gVisor enforces inside the sandbox, which are not the host's
/// `pids.max`: gVisor maps no sandbox process to a host PID. `RLIMIT_NPROC`
/// limits ordinary-user jobs, not sandbox root, so CPU and memory remain the
/// boundary for sudo workloads, and with the limits off there is none: capacity
/// is reserved for the host and the manager rather than treating 16 devices as
/// safe on every VPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub cpus: NonZeroU32,
    pub memory_mib: NonZeroU32,
    /// The Claude Code machine's memory limit.
    pub claude_code_memory_mib: NonZeroU32,
}

impl Limits {
    /// The limits of a sandbox of `machine`: the Claude Code machine's
    /// memory is its own setting.
    pub fn of(self, machine: SandboxMachine) -> Self {
        match machine {
            SandboxMachine::Cloud => self,
            SandboxMachine::ClaudeCode => Self {
                memory_mib: self.claude_code_memory_mib,
                ..self
            },
        }
    }

    /// The cgroup limits of a sandbox whose wake names `named`: what it
    /// names in place of the settings', a number or none at all.
    pub fn sized(self, named: SandboxLimits) -> CgroupLimits {
        let limit = |named: Option<CloudLimit>, setting| named.map_or(Some(setting), CloudLimit::value);
        CgroupLimits {
            cpus: limit(named.cpus, self.cpus),
            memory_mib: limit(named.memory_mib, self.memory_mib),
        }
    }
}

/// One sandbox's cgroup CPU budget and memory limit; none for none, which
/// only an integration's wake names: the sandbox takes the host's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgroupLimits {
    pub cpus: Option<NonZeroU32>,
    pub memory_mib: Option<NonZeroU32>,
}

impl CgroupLimits {
    /// The memory limit in bytes.
    pub fn memory_bytes(&self) -> Option<NonZeroU64> {
        self.memory_mib.map(mebibytes)
    }

    /// Why a sandbox of these limits cannot run on a host of `cpus` CPUs and
    /// `memory_mib` MiB, with each limit beyond the host's; none when it
    /// fits. A cgroup takes a budget over the host's, which would only let
    /// one sandbox take the host from the others.
    pub fn beyond(&self, cpus: u32, memory_mib: u64) -> Option<String> {
        let mut beyond = Vec::new();
        if let Some(budget) = self.cpus
            && budget.get() > cpus
        {
            beyond.push(format!("{budget} CPUs, where the host has {cpus}"));
        }
        if let Some(limit) = self.memory_mib
            && u64::from(limit.get()) > memory_mib
        {
            beyond.push(format!("{limit} MiB of memory, where the host has {memory_mib}"));
        }
        (!beyond.is_empty()).then(|| beyond.join(" and "))
    }
}

/// The host's CPUs and its memory in MiB, from `/proc/meminfo`.
pub fn host_resources() -> std::io::Result<(u32, u64)> {
    let cpus = std::thread::available_parallelism()?.get();
    let meminfo = fs_err::read_to_string("/proc/meminfo")?;
    let kib = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        .ok_or_else(|| std::io::Error::other("/proc/meminfo names no MemTotal"))?;
    Ok((u32::try_from(cpus).unwrap_or(u32::MAX), kib / 1024))
}

/// A setting that stops the manager from starting.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Invalid(#[from] clap::Error),
    #[error("{0} is not a Cloud manager setting")]
    Unknown(String),
    #[error("DEMI_MANAGED_SLOTS exceeds DEMI_MANAGED_SUBNET capacity")]
    SlotsExceedSubnet,
    #[error("DEMI_RELEASE is not set and the executable's directory is unknown: {0}")]
    NoRelease(std::io::Error),
    #[error("{0} applies only with DEMI_MANAGED_LIMITS=on")]
    LimitsOff(&'static str),
}

impl Invocation {
    /// Reads the invocation from the process's arguments and environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::parse(std::env::args_os(), std::env::vars_os())
    }

    /// Reads the invocation from `args`, with the variables in `vars`.
    /// clap reads the process environment itself, so `vars` must be it; tests
    /// pass the same values as arguments.
    pub fn parse(
        args: impl IntoIterator<Item = OsString>,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, ConfigError> {
        let command = Cli::command();
        if let Some(name) =
            demi_shared_cli::unknown_variable(&command, demi_shared_cli::MANAGED_PREFIX, vars)
        {
            return Err(ConfigError::Unknown(name));
        }
        let matches = command.try_get_matches_from(args)?;
        let cli = Cli::from_arg_matches(&matches)?;
        if let Some(directory) = cli.fetch_gvisor {
            return Ok(Self::FetchGvisor(directory));
        }
        let capacity = 1_u64 << (32 - u32::from(cli.subnet.prefix_len()));
        if u64::from(cli.slots) * 4 > capacity {
            return Err(ConfigError::SlotsExceedSubnet);
        }
        let mode = if cli.recover {
            Mode::Recover
        } else if cli.recover_namespace {
            Mode::RecoverNamespace
        } else if cli.check_config {
            Mode::CheckConfig
        } else if cli.check_host {
            Mode::CheckHost
        } else if cli.import {
            Mode::Import
        } else {
            Mode::Serve
        };
        let release = match cli.release {
            Some(release) => release,
            None => release_of_executable()?,
        };
        let runsc = cli
            .runsc
            .unwrap_or_else(|| RuntimeRelease::pinned().directory().join("runsc"));
        let dns = (!cli.dns.is_empty()).then_some(cli.dns);
        let limits = match cli.limits {
            Switch::On => Some(Limits {
                cpus: cli.cpus,
                memory_mib: cli.mem_mib,
                claude_code_memory_mib: cli.claude_code_mem_mib,
            }),
            Switch::Off => {
                // A budget nothing would apply is refused, not ignored.
                for (id, variable) in [
                    ("cpus", "DEMI_MANAGED_CPUS"),
                    ("mem_mib", "DEMI_MANAGED_MEM_MIB"),
                    ("claude_code_mem_mib", "DEMI_MANAGED_CLAUDE_CODE_MEM_MIB"),
                ] {
                    if matches.value_source(id) != Some(ValueSource::DefaultValue) {
                        return Err(ConfigError::LimitsOff(variable));
                    }
                }
                None
            }
        };
        Ok(Self::Manage(Config {
            mode,
            socket: cli.socket,
            data: cli.data,
            runsc,
            proot: cli.proot.unwrap_or_else(|| release.join("bin").join("proot")),
            image: release.join("image"),
            backend_url: cli.backend_url.expect("clap requires the backend's URL without --fetch-gvisor"),
            runner_socket: cli.runner_socket,
            limits,
            subnet: cli.subnet,
            slots: cli.slots,
            dns,
        }))
    }
}

impl Config {
    /// The devices' working pairs: `<data>/working`.
    pub fn working(&self) -> PathBuf {
        self.data.join("working")
    }

    /// Bases and committed generations: `<data>/images`.
    pub fn images(&self) -> PathBuf {
        self.data.join("images")
    }

    /// The integrations' shared folders: `<data>/shared`.
    pub fn shared(&self) -> PathBuf {
        self.data.join("shared")
    }

    /// The runtime directory: `/run/demi-machine-manager`.
    pub fn runtime(&self) -> &Path {
        Path::new(RUNTIME_DIRECTORY)
    }

    /// The directory the manager fetches gVisor's release into, which holds
    /// a directory for each version: none when the configuration names
    /// another runsc than the pinned version's there, which whoever named
    /// it provides.
    pub fn gvisor_directory(&self) -> Option<PathBuf> {
        let pinned = RuntimeRelease::pinned().directory();
        match self.runsc == pinned.join("runsc") {
            true => pinned.parent().map(Path::to_path_buf),
            false => None,
        }
    }
}

/// The server release root this executable lies in: the directory above
/// its own, as `<root>/bin/demi-machine-manager`.
fn release_of_executable() -> Result<PathBuf, ConfigError> {
    let executable = std::env::current_exe().map_err(ConfigError::NoRelease)?;
    executable
        .parent()
        .and_then(Path::parent)
        .map(Path::to_owned)
        .ok_or_else(|| {
            ConfigError::NoRelease(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the executable lies in no directory of a release",
            ))
        })
}

impl Config {
    /// The resolvers a gVisor Cloud, on a network of its own, uses: those
    /// `DEMI_MANAGED_DNS` names, or else those the host itself forwards to,
    /// from the first of [`RESOLVER_FILES`] that exists; or why there are
    /// none, which leaves gVisor unavailable.
    pub fn gvisor_resolvers(&self) -> Result<Vec<Ipv4Addr>, String> {
        if let Some(dns) = &self.dns {
            return Ok(dns.clone());
        }
        let none = || {
            format!(
                "DEMI_MANAGED_DNS is not set and the host names no resolver a gVisor Cloud can use in {}",
                RESOLVER_FILES.join(" or ")
            )
        };
        for path in RESOLVER_FILES {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    let resolvers = usable_resolvers(&text);
                    return if resolvers.is_empty() { Err(none()) } else { Ok(resolvers) };
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(none()),
            }
        }
        Err(none())
    }
}

/// The `nameserver` addresses of a resolv.conf that a sandbox can use: IPv4
/// ones that [`resolver`] accepts. A stub on loopback, such as
/// systemd-resolved's `127.0.0.53`, and IPv6, which the sandbox profile
/// turns off, are left out.
fn usable_resolvers(text: &str) -> Vec<Ipv4Addr> {
    let mut resolvers = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("nameserver") {
            continue;
        }
        if let Some(address) = words.next().and_then(|word| resolver(word).ok())
            && !resolvers.contains(&address)
        {
            resolvers.push(address);
        }
    }
    resolvers
}

fn mebibytes(value: NonZeroU32) -> NonZeroU64 {
    NonZeroU64::from(value)
        .checked_mul(NonZeroU64::new(1 << 20).expect("a MiB is not zero"))
        .expect("a u32 count of MiB fits in u64 bytes")
}

/// Refuses a `DEMI_MANAGED_*` variable the command does not declare. The
/// declared names come from the command itself, so they are listed once.
fn absolute(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("must be an absolute path".into());
    }
    Ok(path)
}

/// An absolute path in a directory of its own, named as a Cloud's runner
/// looks for it.
fn runner_socket(value: &str) -> Result<PathBuf, String> {
    let path = absolute(value)?;
    let expected = Path::new(demi_runner_protocol::boot::BACKEND_SOCKET);
    if path.file_name() != expected.file_name() || path.parent() == Some(Path::new("/")) {
        return Err(format!(
            "must be a file named {} in a directory of its own",
            expected.file_name().unwrap_or_default().display()
        ));
    }
    Ok(path)
}

fn backend_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("must be an http or https URL".into());
    }
    Ok(url)
}

/// A positive count or size written in decimal digits only: `0x10`, `1e3`,
/// `+12` and ` 12 ` are refused.
fn decimal<T: FromStr>(value: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must be a positive decimal integer".into());
    }
    value.parse().map_err(|error: T::Err| error.to_string())
}

fn subnet(value: &str) -> Result<Ipv4Net, String> {
    let net = Ipv4Net::from_str(value).map_err(|error| error.to_string())?;
    if !(8..=30).contains(&net.prefix_len()) || net.addr() != net.network() {
        return Err("must be an aligned IPv4 network with prefix /8 through /30".into());
    }
    Ok(net)
}

fn slots(value: &str) -> Result<u16, String> {
    let count: u16 = decimal(value)?;
    if !(1..=16384).contains(&count) {
        return Err("must be from 1 to 16384".into());
    }
    Ok(count)
}

/// A resolver a sandbox can reach: not unspecified (`0.0.0.0/8`), loopback,
/// multicast or broadcast.
fn resolver(value: &str) -> Result<Ipv4Addr, String> {
    let address = Ipv4Addr::from_str(value).map_err(|error| error.to_string())?;
    if address.octets()[0] == 0
        || address.is_loopback()
        || address.is_multicast()
        || address.is_broadcast()
    {
        return Err("resolver must be reachable IPv4".into());
    }
    Ok(address)
}
