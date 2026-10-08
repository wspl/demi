//! The manager's configuration (`setup.md` § Configuration): its command line,
//! its own `DEMI_MANAGED_*` variables and the backend's settings it shares
//! (`DEMI_RELEASE`, `DEMI_BACKEND_PUBLIC_URL`, `DEMI_MACHINE_MANAGER_SOCKET`),
//! all read from the deployment's one configuration file and validated at
//! startup. An invalid or unknown setting stops the manager with an error
//! that names the variable.

use std::{
    ffi::OsString,
    net::Ipv4Addr,
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
    str::FromStr,
};

use clap::{CommandFactory, FromArgMatches, parser::ValueSource};
use demi_machine_manager_protocol::runtime::RuntimeRelease;
use ipnet::Ipv4Net;

/// Where the manager keeps its runtime bundles, locks and namespace handle.
pub const RUNTIME_DIRECTORY: &str = "/run/demi-machine-manager";

/// The socket the manager listens on unless `DEMI_MACHINE_MANAGER_SOCKET`
/// names another, the backend's default too.
const SOCKET: &str = "/run/demi-cloud/machines.sock";

/// Where the host's own resolvers are named: systemd-resolved's upstream
/// file, which a host running it has, else the classic one.
const RESOLVER_FILES: [&str; 2] = ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"];

/// The Cloud machine manager: runs users' Cloud machines as gVisor
/// sandboxes and serves the backend over a Unix socket.
#[derive(Debug, clap::Parser)]
#[command(name = "demi-machine-manager", version = demi_shared_artifacts::WORKSPACE_VERSION)]
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
    /// The backend's public URL: the only backend a sandbox's runner may
    /// connect to.
    #[arg(
        long,
        env = "DEMI_BACKEND_PUBLIC_URL",
        value_name = "DEMI_BACKEND_PUBLIC_URL",
        value_parser = backend_url
    )]
    backend_url: url::Url,
    /// Whether sandboxes run under cgroup v2 CPU, memory and PID limits.
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
    /// A new system filesystem's capacity, in MiB.
    #[arg(long, env = "DEMI_MANAGED_SYSTEM_MIB", value_name = "DEMI_MANAGED_SYSTEM_MIB", default_value = "1024", value_parser = decimal::<NonZeroU32>)]
    system_mib: NonZeroU32,
    /// A new home filesystem's capacity, in MiB.
    #[arg(long, env = "DEMI_MANAGED_HOME_MIB", value_name = "DEMI_MANAGED_HOME_MIB", default_value = "1024", value_parser = decimal::<NonZeroU32>)]
    home_mib: NonZeroU32,
    /// The IPv4 pool the sandboxes' networks come from.
    #[arg(long, env = "DEMI_MANAGED_SUBNET", value_name = "DEMI_MANAGED_SUBNET", default_value = "172.30.0.0/16", value_parser = subnet)]
    subnet: Ipv4Net,
    /// How many sandboxes may run at once; each takes four addresses.
    #[arg(long, env = "DEMI_MANAGED_SLOTS", value_name = "DEMI_MANAGED_SLOTS", default_value = "256", value_parser = slots)]
    slots: u16,
    /// The resolvers a sandbox uses, separated by commas [default: the
    /// host's own upstream resolvers].
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

/// What the manager was started to do.
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
    pub image: PathBuf,
    pub backend_url: url::Url,
    /// Each sandbox's cgroup limits; `None` with `DEMI_MANAGED_LIMITS=off`,
    /// which runs sandboxes without cgroups.
    pub limits: Option<Limits>,
    pub system_mib: NonZeroU32,
    pub home_mib: NonZeroU32,
    pub subnet: Ipv4Net,
    pub slots: u16,
    pub dns: Vec<Ipv4Addr>,
}

/// One sandbox's cgroup v2 limits (`managed-hosts.md` § Resource limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub cpus: NonZeroU32,
    pub memory_mib: NonZeroU32,
}

impl Limits {
    /// The memory limit in bytes.
    pub fn memory_bytes(&self) -> NonZeroU64 {
        mebibytes(self.memory_mib)
    }
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
    #[error(
        "DEMI_MANAGED_DNS is not set and the host names no resolver a sandbox can use in {}",
        RESOLVER_FILES.join(" or ")
    )]
    NoResolver,
    #[error("{0} applies only with DEMI_MANAGED_LIMITS=on")]
    LimitsOff(&'static str),
}

impl Config {
    /// Reads the configuration from the process's arguments and environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::parse(std::env::args_os(), std::env::vars_os())
    }

    /// Reads the configuration from `args`, with the variables in `vars`.
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
        let dns = if cli.dns.is_empty() {
            host_resolvers()?
        } else {
            cli.dns
        };
        let limits = match cli.limits {
            Switch::On => Some(Limits {
                cpus: cli.cpus,
                memory_mib: cli.mem_mib,
            }),
            Switch::Off => {
                // A budget nothing would apply is refused, not ignored.
                for (id, variable) in [
                    ("cpus", "DEMI_MANAGED_CPUS"),
                    ("mem_mib", "DEMI_MANAGED_MEM_MIB"),
                ] {
                    if matches.value_source(id) != Some(ValueSource::DefaultValue) {
                        return Err(ConfigError::LimitsOff(variable));
                    }
                }
                None
            }
        };
        Ok(Self {
            mode,
            socket: cli.socket,
            data: cli.data,
            runsc,
            image: release.join("image"),
            backend_url: cli.backend_url,
            limits,
            system_mib: cli.system_mib,
            home_mib: cli.home_mib,
            subnet: cli.subnet,
            slots: cli.slots,
            dns,
        })
    }

    /// The devices' working pairs: `<data>/working`.
    pub fn working(&self) -> PathBuf {
        self.data.join("working")
    }

    /// Bases and committed generations: `<data>/images`.
    pub fn images(&self) -> PathBuf {
        self.data.join("images")
    }

    /// The runtime directory: `/run/demi-machine-manager`.
    pub fn runtime(&self) -> &Path {
        Path::new(RUNTIME_DIRECTORY)
    }

    /// A new system filesystem's capacity in bytes.
    pub fn system_bytes(&self) -> NonZeroU64 {
        mebibytes(self.system_mib)
    }

    /// A new home filesystem's capacity in bytes.
    pub fn home_bytes(&self) -> NonZeroU64 {
        mebibytes(self.home_mib)
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

/// The resolvers the host itself forwards to, from the first of
/// [`RESOLVER_FILES`] that exists.
fn host_resolvers() -> Result<Vec<Ipv4Addr>, ConfigError> {
    for path in RESOLVER_FILES {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let resolvers = usable_resolvers(&text);
                if resolvers.is_empty() {
                    return Err(ConfigError::NoResolver);
                }
                return Ok(resolvers);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ConfigError::NoResolver),
        }
    }
    Err(ConfigError::NoResolver)
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

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: [(&str, &str); 4] = [
        ("DEMI_MANAGED_RUNSC", "/opt/gvisor/runsc"),
        ("DEMI_RELEASE", "/opt/demi/0.1.3"),
        ("DEMI_BACKEND_PUBLIC_URL", "https://backend.example.com"),
        ("DEMI_MANAGED_DNS", "1.1.1.1,8.8.8.8"),
    ];

    /// Parses `settings` over the required ones, each passed as its flag,
    /// with the variables present in the environment the check reads.
    fn parse(mode: &[&str], settings: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut values: Vec<(String, String)> = REQUIRED
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        values.push((
            "DEMI_MACHINE_MANAGER_SOCKET".into(),
            "/run/demi-cloud/machines.sock".into(),
        ));
        for (name, value) in settings {
            values.retain(|(existing, _)| existing != name);
            values.push(((*name).to_owned(), (*value).to_owned()));
        }
        let command = Cli::command();
        let mut args = vec![OsString::from("demi-machine-manager")];
        args.extend(mode.iter().map(OsString::from));
        let vars: Vec<_> = values
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect();
        for (name, value) in &values {
            let flag = command
                .get_arguments()
                .find(|argument| argument.get_env().is_some_and(|env| env == name.as_str()));
            if let Some(flag) = flag {
                args.push(format!("--{}={value}", flag.get_long().expect("a long flag")).into());
            }
        }
        Config::parse(args, vars)
    }

    #[test]
    fn defaults_apply_and_the_backend_url_is_normalized() {
        let config = parse(&[], &[]).expect("valid configuration");
        assert_eq!(config.mode, Mode::Serve);
        assert_eq!(config.backend_url.as_str(), "https://backend.example.com/");
        assert_eq!(config.image, PathBuf::from("/opt/demi/0.1.3/image"));
        assert_eq!(config.data, PathBuf::from("/opt/demi/data/cloud"));
        assert_eq!(
            config.working(),
            PathBuf::from("/opt/demi/data/cloud/working")
        );
        let limits = config.limits.expect("the limits are on by default");
        assert_eq!(limits.cpus.get(), 2);
        assert_eq!(limits.memory_bytes().get(), 2 << 30);
        assert_eq!(config.system_bytes().get(), 1 << 30);
        assert_eq!(config.home_bytes().get(), 1 << 30);
        assert_eq!(config.subnet.to_string(), "172.30.0.0/16");
        assert_eq!(config.slots, 256);
        assert_eq!(
            config.dns,
            [Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)]
        );
    }

    #[test]
    fn obsolete_malformed_and_insufficient_settings_are_refused() {
        let refused: [&[(&str, &str)]; 21] = [
            &[("DEMI_MANAGED_FIRECRACKER", "/old")],
            &[("DEMI_MANAGED_SUBNET", "172.30.1.0/16")],
            &[("DEMI_MANAGED_SUBNET", "172.30.0.0/31")],
            &[("DEMI_MANAGED_SUBNET", "10.0.0.0/7")],
            // Two slots need eight addresses; a /30 holds four.
            &[
                ("DEMI_MANAGED_SUBNET", "172.30.0.0/30"),
                ("DEMI_MANAGED_SLOTS", "2"),
            ],
            &[("DEMI_MANAGED_DNS", "127.0.0.1")],
            &[("DEMI_MANAGED_DNS", "0.1.2.3")],
            &[("DEMI_MANAGED_DNS", "224.0.0.1")],
            &[("DEMI_MANAGED_DNS", "255.255.255.255")],
            &[("DEMI_MANAGED_DNS", "1.1.1.1,")],
            &[("DEMI_MANAGED_CPUS", "0")],
            &[("DEMI_MANAGED_CPUS", "0x10")],
            &[("DEMI_MANAGED_MEM_MIB", "1e3")],
            &[("DEMI_MANAGED_SYSTEM_MIB", " 12 ")],
            &[("DEMI_MANAGED_HOME_MIB", "+12")],
            &[("DEMI_MANAGED_SLOTS", "16385")],
            &[("DEMI_MANAGED_RUNSC", "runsc")],
            &[("DEMI_MANAGED_DATA", "state")],
            &[("DEMI_RELEASE", "release")],
            &[("DEMI_BACKEND_PUBLIC_URL", "file:///tmp/backend")],
            &[("DEMI_MANAGED_LIMITS", "yes")],
        ];
        for settings in refused {
            assert!(parse(&[], settings).is_err(), "{settings:?} was accepted");
        }
    }

    #[test]
    fn with_the_limits_off_sandboxes_have_none_and_a_budget_is_refused() {
        let off = parse(&[], &[("DEMI_MANAGED_LIMITS", "off")]).expect("the limits off");
        assert_eq!(off.limits, None);
        for variable in ["DEMI_MANAGED_CPUS", "DEMI_MANAGED_MEM_MIB"] {
            // The default value, written out, is still a budget nothing applies.
            let value = if variable == "DEMI_MANAGED_CPUS" {
                "2"
            } else {
                "2048"
            };
            let error = parse(&[], &[("DEMI_MANAGED_LIMITS", "off"), (variable, value)])
                .expect_err("refused");
            assert_eq!(
                error.to_string(),
                format!("{variable} applies only with DEMI_MANAGED_LIMITS=on")
            );
        }
    }

    #[test]
    fn an_unknown_managed_variable_is_named() {
        let error = parse(&[], &[("DEMI_MANAGED_FIRECRACKER", "/old")]).expect_err("refused");
        assert_eq!(
            error.to_string(),
            "DEMI_MANAGED_FIRECRACKER is not a Cloud manager setting"
        );
    }

    #[test]
    fn the_socket_and_the_runsc_default_to_the_backends_and_the_pinned_version() {
        let command = Cli::command();
        let mut args = vec![OsString::from("demi-machine-manager"), "--recover".into()];
        for (name, value) in REQUIRED
            .into_iter()
            .filter(|(name, _)| *name != "DEMI_MANAGED_RUNSC")
        {
            let flag = command
                .get_arguments()
                .find(|argument| argument.get_env().is_some_and(|env| env == name))
                .and_then(clap::Arg::get_long)
                .expect("a flag");
            args.push(format!("--{flag}={value}").into());
        }
        let recovering = Config::parse(args, Vec::new()).expect("valid configuration");
        assert_eq!(recovering.mode, Mode::Recover);
        assert_eq!(recovering.socket, PathBuf::from(SOCKET));
        assert_eq!(
            recovering.runsc,
            Path::new("/opt/demi/gvisor")
                .join(RuntimeRelease::pinned().version())
                .join("runsc")
        );
        assert!(parse(&["--recover", "--recover-namespace"], &[]).is_err());
    }

    #[test]
    fn the_hosts_resolvers_are_the_ones_a_sandbox_can_use() {
        let conf = "# systemd-resolved\nnameserver 127.0.0.53\nnameserver 185.12.64.1\n\
                    nameserver 2a01:4ff:ff00::add:1\nnameserver 185.12.64.2\nnameserver 185.12.64.1\n\
                    search example.test\n";
        assert_eq!(
            usable_resolvers(conf),
            [Ipv4Addr::new(185, 12, 64, 1), Ipv4Addr::new(185, 12, 64, 2)]
        );
        assert!(usable_resolvers("nameserver 127.0.0.53\noptions edns0\n").is_empty());
    }

    /// The release's unit, which the installer
    /// (`scripts/install-managed-hosts.sh`) puts beneath a staging root with
    /// the link to the release: systemd accepts the unit, and the settings
    /// configure this manager.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs root: run the Linux suite with --ignored as root"]
    fn the_installer_writes_a_unit_systemd_accepts_and_settings_this_manager_reads() {
        use std::{os::unix::fs::PermissionsExt, process::Command};

        use crate::{
            blocking::OffLoop,
            linux::{loopdev, mount, testing::isolate},
        };

        isolate();
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        // The state directory lives on its own ext4 filesystem.
        let image = directory.path().join("data.img");
        std::fs::File::create(&image)
            .unwrap()
            .set_len(64 << 20)
            .unwrap();
        let formatted = Command::new("mkfs.ext4")
            .args(["-q", "-F"])
            .arg(&image)
            .status()
            .unwrap();
        assert!(formatted.success());
        let data = directory.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let device = loopdev::attach(&off, &image).unwrap();
        mount::ext4(&off, &device.path(), &data).unwrap();
        drop(device);
        // A server release with its manager and its image.
        let release = directory.path().join("release");
        std::fs::create_dir_all(release.join("bin")).unwrap();
        std::fs::create_dir_all(release.join("image")).unwrap();
        std::fs::write(release.join("image/manifest.json"), "{}").unwrap();
        std::fs::create_dir_all(release.join("systemd")).unwrap();
        let program = release.join("bin/demi-machine-manager");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let shipped = include_str!("../systemd/demi-machine-manager.service");
        std::fs::write(release.join("systemd/demi-machine-manager.service"), shipped).unwrap();
        // The deployment's configuration file, which the backend reads too.
        let root = directory.path().join("root");
        let settings = root.join("opt/demi/config/demi.env");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            format!(
                "DEMI_BACKEND_PUBLIC_URL=https://backend.example.com\n\
                 DEMI_INSTANCE_MODE=isolated\n\
                 DEMI_MANAGED_DATA={}\n\
                 DEMI_MANAGED_DNS=1.1.1.1,8.8.8.8\n\
                 DEMI_MANAGED_SLOTS=16\n\
                 DEMI_MANAGED_LIMITS=off\n",
                data.display()
            ),
        )
        .unwrap();
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/install-managed-hosts.sh"
        );
        let installed = Command::new("bash")
            .arg(script)
            .arg("--root")
            .arg(&root)
            .args(["--user", "root", "--release"])
            .arg(&release)
            .output()
            .unwrap();
        assert!(
            installed.status.success(),
            "{}",
            String::from_utf8_lossy(&installed.stderr)
        );

        let unit_path = root.join("etc/systemd/system/demi-machine-manager.service");
        let unit = std::fs::read_to_string(&unit_path).unwrap();
        assert_eq!(unit, shipped);
        assert_eq!(std::fs::read_link(root.join("opt/demi/current")).unwrap(), release);
        let manager = "/opt/demi/current/bin/demi-machine-manager";
        for directive in [
            "Type=notify".to_owned(),
            "KillMode=mixed".to_owned(),
            "TimeoutStartSec=infinity".to_owned(),
            "TimeoutStopSec=infinity".to_owned(),
            "PrivateMounts=yes".to_owned(),
            "UMask=0077".to_owned(),
            "Group=demi-cloud".to_owned(),
            "EnvironmentFile=/opt/demi/config/demi.env".to_owned(),
            format!("ExecStart={manager}"),
            format!("ExecStopPost={manager} --recover"),
        ] {
            assert!(
                unit.lines().any(|line| line == directive),
                "{directive} is missing:\n{unit}"
            );
        }
        // systemd checks the unit as a server runs it, with /opt/demi/current
        // at the release, here on a tmpfs of this test's own mount namespace.
        let opt = Path::new("/opt");
        std::fs::create_dir_all(opt).unwrap();
        mount::tmpfs(&off, opt, "mode=0755").unwrap();
        std::fs::create_dir(opt.join("demi")).unwrap();
        std::os::unix::fs::symlink(&release, opt.join("demi/current")).unwrap();
        let verified = Command::new("systemd-analyze")
            .arg("verify")
            .arg(&unit_path)
            .output()
            .unwrap();
        let warnings = String::from_utf8_lossy(&verified.stderr);
        assert!(verified.status.success(), "{warnings}");
        assert!(
            !warnings.contains("demi-machine-manager.service"),
            "{warnings}"
        );

        // The file configures this manager: each of the manager's settings
        // passed as the flag clap gives it, the backend's own left to the
        // backend, and the release the one the manager's executable lies in.
        let command = Cli::command();
        let mut args = vec![
            OsString::from("demi-machine-manager"),
            format!("--release={}", release.display()).into(),
        ];
        let mut vars = Vec::new();
        for line in std::fs::read_to_string(&settings).unwrap().lines() {
            let (name, value) = line.split_once('=').expect("a setting is NAME=VALUE");
            vars.push((OsString::from(name), OsString::from(value)));
            let flag = command
                .get_arguments()
                .find(|argument| argument.get_env().is_some_and(|env| env == name))
                .and_then(clap::Arg::get_long);
            if let Some(flag) = flag {
                args.push(format!("--{flag}={value}").into());
            }
        }
        let config = Config::parse(args, vars).expect("the installed settings");
        assert_eq!(config.mode, Mode::Serve);
        assert_eq!(config.data, data);
        assert_eq!(config.image, release.join("image"));
        assert_eq!(config.socket, PathBuf::from(SOCKET));
        assert_eq!(config.backend_url.as_str(), "https://backend.example.com/");
        assert_eq!(
            config.dns,
            [Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)]
        );
        assert_eq!(config.slots, 16);
        assert_eq!(config.limits, None);
        mount::unmount(&off, opt).unwrap();
        mount::unmount(&off, &data).unwrap();
    }
}
