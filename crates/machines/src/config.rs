//! The manager's configuration (`setup.md` § Configuration): its command line
//! and the `DEMI_MACHINES_*` and `DEMI_MANAGED_*` variables, validated at
//! startup. An invalid or unknown setting stops the manager with an error
//! that names the variable.

use std::{
    ffi::OsString,
    net::Ipv4Addr,
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
    str::FromStr,
};

use clap::{CommandFactory, FromArgMatches};
use ipnet::Ipv4Net;

/// Where the manager keeps its runtime bundles, locks and namespace handle.
pub const RUNTIME_DIRECTORY: &str = "/run/demi-machines";

/// The prefix of the variables the manager owns: one it does not know is an
/// error, so a setting from another runtime cannot be silently ignored.
const MANAGED_PREFIX: &str = "DEMI_MANAGED_";

/// The Cloud machine manager: runs users' Cloud machines as gVisor
/// sandboxes and serves the backend over a Unix socket.
#[derive(Debug, clap::Parser)]
#[command(name = "demi-machines", version)]
struct Cli {
    /// Fence and save what a stopped manager left behind, then exit (the
    /// service's stop-post command).
    #[arg(long, conflicts_with = "recover_namespace")]
    recover: bool,
    /// Recover inside a saved mount namespace; only the manager starts this.
    #[arg(long, hide = true)]
    recover_namespace: bool,
    /// The socket the backend connects to; required to serve.
    #[arg(long, env = "DEMI_MACHINES_SOCKET", value_name = "DEMI_MACHINES_SOCKET", value_parser = absolute)]
    socket: Option<PathBuf>,
    /// The persistent state directory, on one filesystem.
    #[arg(
        long,
        env = "DEMI_MACHINES_DATA",
        value_name = "DEMI_MACHINES_DATA",
        default_value = "/var/lib/demi-machines",
        value_parser = absolute
    )]
    data: PathBuf,
    /// The pinned runsc executable.
    #[arg(long, env = "DEMI_MANAGED_RUNSC", value_name = "DEMI_MANAGED_RUNSC", value_parser = absolute)]
    runsc: PathBuf,
    /// The directory holding the Cloud image manifest and archive.
    #[arg(long, env = "DEMI_MANAGED_IMAGE", value_name = "DEMI_MANAGED_IMAGE", value_parser = absolute)]
    image: PathBuf,
    /// The only backend a sandbox's runner may connect to.
    #[arg(
        long,
        env = "DEMI_MANAGED_BACKEND_URL",
        value_name = "DEMI_MANAGED_BACKEND_URL",
        value_parser = backend_url
    )]
    backend_url: url::Url,
    /// The CPU budget of one sandbox.
    #[arg(long, env = "DEMI_MANAGED_CPUS", value_name = "DEMI_MANAGED_CPUS", default_value = "2", value_parser = decimal::<NonZeroU32>)]
    cpus: NonZeroU32,
    /// The memory limit of one sandbox, in MiB.
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
    /// The resolvers a sandbox uses, separated by commas.
    #[arg(
        long,
        env = "DEMI_MANAGED_DNS",
        value_name = "DEMI_MANAGED_DNS",
        value_delimiter = ',',
        required = true,
        value_parser = resolver
    )]
    dns: Vec<Ipv4Addr>,
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
}

/// The validated configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub mode: Mode,
    /// Present in [`Mode::Serve`].
    pub socket: Option<PathBuf>,
    pub data: PathBuf,
    pub runsc: PathBuf,
    pub image: PathBuf,
    pub backend_url: url::Url,
    pub cpus: NonZeroU32,
    pub memory_mib: NonZeroU32,
    pub system_mib: NonZeroU32,
    pub home_mib: NonZeroU32,
    pub subnet: Ipv4Net,
    pub slots: u16,
    pub dns: Vec<Ipv4Addr>,
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
    #[error("DEMI_MACHINES_SOCKET is required")]
    MissingSocket,
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
        reject_unknown_managed_vars(&command, vars)?;
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
        } else {
            Mode::Serve
        };
        if mode == Mode::Serve && cli.socket.is_none() {
            return Err(ConfigError::MissingSocket);
        }
        Ok(Self {
            mode,
            socket: cli.socket,
            data: cli.data,
            runsc: cli.runsc,
            image: cli.image,
            backend_url: cli.backend_url,
            cpus: cli.cpus,
            memory_mib: cli.mem_mib,
            system_mib: cli.system_mib,
            home_mib: cli.home_mib,
            subnet: cli.subnet,
            slots: cli.slots,
            dns: cli.dns,
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

    /// The runtime directory: `/run/demi-machines`.
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

    /// One sandbox's memory limit in bytes.
    pub fn memory_bytes(&self) -> NonZeroU64 {
        mebibytes(self.memory_mib)
    }
}

fn mebibytes(value: NonZeroU32) -> NonZeroU64 {
    NonZeroU64::from(value)
        .checked_mul(NonZeroU64::new(1 << 20).expect("a MiB is not zero"))
        .expect("a u32 count of MiB fits in u64 bytes")
}

/// Refuses a `DEMI_MANAGED_*` variable the command does not declare. The
/// declared names come from the command itself, so they are listed once.
fn reject_unknown_managed_vars(
    command: &clap::Command,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<(), ConfigError> {
    let known: Vec<_> = command
        .get_arguments()
        .filter_map(clap::Arg::get_env)
        .collect();
    for (name, _) in vars {
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(MANAGED_PREFIX) && !known.iter().any(|known| *known == name) {
            return Err(ConfigError::Unknown(name.to_owned()));
        }
    }
    Ok(())
}

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
    if address.octets()[0] == 0 || address.is_loopback() || address.is_multicast() || address.is_broadcast()
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
        ("DEMI_MANAGED_IMAGE", "/opt/image"),
        ("DEMI_MANAGED_BACKEND_URL", "https://backend.example.com"),
        ("DEMI_MANAGED_DNS", "1.1.1.1,8.8.8.8"),
    ];

    /// Parses `settings` over the required ones, each passed as its flag,
    /// with the variables present in the environment the check reads.
    fn parse(mode: &[&str], settings: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut values: Vec<(String, String)> = REQUIRED
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        values.push(("DEMI_MACHINES_SOCKET".into(), "/run/demi-cloud/machines.sock".into()));
        for (name, value) in settings {
            values.retain(|(existing, _)| existing != name);
            values.push(((*name).to_owned(), (*value).to_owned()));
        }
        let command = Cli::command();
        let mut args = vec![OsString::from("demi-machines")];
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
        assert_eq!(config.data, PathBuf::from("/var/lib/demi-machines"));
        assert_eq!(config.working(), PathBuf::from("/var/lib/demi-machines/working"));
        assert_eq!(config.cpus.get(), 2);
        assert_eq!(config.memory_bytes().get(), 2 << 30);
        assert_eq!(config.system_bytes().get(), 1 << 30);
        assert_eq!(config.home_bytes().get(), 1 << 30);
        assert_eq!(config.subnet.to_string(), "172.30.0.0/16");
        assert_eq!(config.slots, 256);
        assert_eq!(config.dns, [Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)]);
    }

    #[test]
    fn obsolete_malformed_and_insufficient_settings_are_refused() {
        for settings in [
            [("DEMI_MANAGED_FIRECRACKER", "/old")],
            [("DEMI_MANAGED_SUBNET", "172.30.1.0/16")],
            [("DEMI_MANAGED_SUBNET", "172.30.0.0/31")],
            [("DEMI_MANAGED_SUBNET", "10.0.0.0/7")],
            [("DEMI_MANAGED_SLOTS", "2")],
            [("DEMI_MANAGED_DNS", "127.0.0.1")],
            [("DEMI_MANAGED_DNS", "0.1.2.3")],
            [("DEMI_MANAGED_DNS", "224.0.0.1")],
            [("DEMI_MANAGED_DNS", "255.255.255.255")],
            [("DEMI_MANAGED_DNS", "1.1.1.1,")],
            [("DEMI_MANAGED_CPUS", "0")],
            [("DEMI_MANAGED_CPUS", "0x10")],
            [("DEMI_MANAGED_MEM_MIB", "1e3")],
            [("DEMI_MANAGED_SYSTEM_MIB", " 12 ")],
            [("DEMI_MANAGED_HOME_MIB", "+12")],
            [("DEMI_MANAGED_SLOTS", "16385")],
            [("DEMI_MANAGED_RUNSC", "runsc")],
            [("DEMI_MACHINES_DATA", "state")],
            [("DEMI_MANAGED_BACKEND_URL", "file:///tmp/backend")],
        ] {
            let settings = if settings[0].0 == "DEMI_MANAGED_SLOTS" && settings[0].1 == "2" {
                // Two slots need eight addresses; a /30 holds four.
                vec![("DEMI_MANAGED_SUBNET", "172.30.0.0/30"), settings[0]]
            } else {
                settings.to_vec()
            };
            assert!(parse(&[], &settings).is_err(), "{settings:?} was accepted");
        }
    }

    #[test]
    fn an_unknown_managed_variable_is_named() {
        let error = parse(&[], &[("DEMI_MANAGED_FIRECRACKER", "/old")]).expect_err("refused");
        assert_eq!(error.to_string(), "DEMI_MANAGED_FIRECRACKER is not a Cloud manager setting");
    }

    #[test]
    fn serving_needs_a_socket_and_recovery_does_not() {
        let command = Cli::command();
        let mut args = vec![OsString::from("demi-machines"), "--recover".into()];
        for (name, value) in REQUIRED {
            let flag = command
                .get_arguments()
                .find(|argument| argument.get_env().is_some_and(|env| env == name))
                .and_then(clap::Arg::get_long)
                .expect("a flag");
            args.push(format!("--{flag}={value}").into());
        }
        let recovering = Config::parse(args.clone(), Vec::new()).expect("recovery needs no socket");
        assert_eq!(recovering.mode, Mode::Recover);
        args.remove(1);
        assert!(matches!(Config::parse(args, Vec::new()), Err(ConfigError::MissingSocket)));
        assert!(parse(&["--recover", "--recover-namespace"], &[]).is_err());
    }

    /// The installer's unit and settings (`scripts/install-managed-hosts.sh`),
    /// written beneath a staging root: systemd accepts the unit, and the
    /// settings configure this manager.
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
        std::fs::File::create(&image).unwrap().set_len(64 << 20).unwrap();
        let formatted = Command::new("mkfs.ext4").args(["-q", "-F"]).arg(&image).status().unwrap();
        assert!(formatted.success());
        let data = directory.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let device = loopdev::attach(&off, &image).unwrap();
        mount::ext4(&off, &device.path(), &data).unwrap();
        drop(device);
        let release = directory.path().join("image");
        std::fs::create_dir(&release).unwrap();
        std::fs::write(release.join("manifest.json"), "{}").unwrap();
        let manager = directory.path().join("demi-machines");
        std::fs::write(&manager, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&manager, std::fs::Permissions::from_mode(0o755)).unwrap();
        let root = directory.path().join("root");
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/install-managed-hosts.sh");
        let installed = Command::new("bash")
            .arg(script)
            .arg("--root")
            .arg(&root)
            .args(["--user", "root", "--manager"])
            .arg(&manager)
            .arg("--image")
            .arg(&release)
            .args(["--backend-url", "https://backend.example.com", "--dns", "1.1.1.1,8.8.8.8", "--data"])
            .arg(&data)
            .args(["--slots", "16"])
            .output()
            .unwrap();
        assert!(installed.status.success(), "{}", String::from_utf8_lossy(&installed.stderr));

        let unit_path = root.join("etc/systemd/system/demi-machines.service");
        let unit = std::fs::read_to_string(&unit_path).unwrap();
        let manager = manager.display();
        for directive in [
            "Type=notify".to_owned(),
            "KillMode=mixed".to_owned(),
            "TimeoutStartSec=infinity".to_owned(),
            "TimeoutStopSec=infinity".to_owned(),
            "PrivateMounts=yes".to_owned(),
            "UMask=0077".to_owned(),
            "Group=demi-cloud".to_owned(),
            "EnvironmentFile=/etc/demi-machines/manager.env".to_owned(),
            format!("ExecStart={manager}"),
            format!("ExecStopPost={manager} --recover"),
        ] {
            assert!(unit.lines().any(|line| line == directive), "{directive} is missing:\n{unit}");
        }
        let verified = Command::new("systemd-analyze").arg("verify").arg(&unit_path).output().unwrap();
        let warnings = String::from_utf8_lossy(&verified.stderr);
        assert!(verified.status.success(), "{warnings}");
        assert!(!warnings.contains("demi-machines.service"), "{warnings}");

        // Each setting is one this manager knows, passed as the flag clap
        // gives it, and together they configure the manager.
        let settings = std::fs::read_to_string(root.join("etc/demi-machines/manager.env")).unwrap();
        let command = Cli::command();
        let mut args = vec![OsString::from("demi-machines")];
        let mut vars = Vec::new();
        for line in settings.lines() {
            let (name, value) = line.split_once('=').expect("a setting is NAME=VALUE");
            let flag = command
                .get_arguments()
                .find(|argument| argument.get_env().is_some_and(|env| env == name))
                .and_then(clap::Arg::get_long)
                .unwrap_or_else(|| panic!("{name} is not a setting of this manager"));
            args.push(format!("--{flag}={value}").into());
            vars.push((OsString::from(name), OsString::from(value)));
        }
        let config = Config::parse(args, vars).expect("the installed settings");
        assert_eq!(config.mode, Mode::Serve);
        assert_eq!(config.data, data);
        assert_eq!(config.image, release);
        assert_eq!(config.socket.as_deref(), Some(Path::new("/run/demi-cloud/machines.sock")));
        assert_eq!(config.backend_url.as_str(), "https://backend.example.com/");
        assert_eq!(config.dns, [Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(config.slots, 16);
        assert!(config.runsc.starts_with("/opt/gvisor"), "{}", config.runsc.display());
        mount::unmount(&off, &data).unwrap();
    }
}
