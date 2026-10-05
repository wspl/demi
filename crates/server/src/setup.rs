//! `demi-server setup` (`installation.md`): sets a Linux machine up as a Demi
//! server with the release of this program, from the choices its parameters
//! give. A person at a terminal is asked for what is missing; without a
//! terminal, as an agent runs it, a missing choice prints the guide. Every
//! step can be taken again, so the same command continues a run that stopped,
//! such as for a proxy that was not ready yet.

use std::{
    io::{self, BufRead as _, IsTerminal as _, Write as _},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
};

use demi_shared_artifacts::{Mode as Publish, Permissions, Publication};
use semver::Version;
use tokio_util::sync::CancellationToken;

use crate::{
    fetch,
    layout::{Layout, UNITS},
    services::Services,
    settings::Settings,
};

/// The guide `--help` prints, and what a run without a terminal and without
/// a required choice prints.
pub const GUIDE: &str = "\
Sets this Linux machine up as a Demi server, with the release of this
demi-server, then checks it from outside. Run as root.

Demi serves plain HTTP and never terminates TLS: something in front of it
serves the domain over HTTPS and forwards to the address --listen names. It
must pass the Origin and Host headers unchanged and allow WebSocket upgrades.
  - A reverse proxy on this machine (Caddy, nginx) or a tunnel (such as
    Cloudflare Tunnel, for a machine without a public address):
      --listen 127.0.0.1:3271
  - Cloudflare's proxy, which reaches the machine from outside, on a port
    Cloudflare forwards HTTP to:
      --listen 0.0.0.0:8080

The machine must be able to run Cloud: setup checks it with the release's
machine manager and refuses one that cannot. The first visitor of the domain
afterwards creates the master account.

Examples:
  demi-server setup --domain demi.example.com --mode isolated --listen 127.0.0.1:3271
  demi-server setup --domain demi.example.com --mode shared --listen 0.0.0.0:8080 \\
    --expose-domain expose-example.com
  AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... demi-server setup \\
    --domain demi.example.com --mode isolated --listen 127.0.0.1:3271 \\
    --storage s3 --s3-bucket demi --s3-region eu-central-1

A run that stops, such as before the proxy is ready, says what to do; the
same command then continues where it stopped. A server that is set up moves
to later releases with `demi-server upgrade`.";

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum InstanceMode {
    Shared,
    Isolated,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Storage {
    Local,
    S3,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Switch {
    On,
    Off,
}

#[derive(clap::Args)]
pub struct Options {
    /// The domain Demi is reached at, over HTTPS: the public URL is
    /// https://<domain>. Required.
    #[arg(long, value_name = "NAME")]
    domain: Option<String>,
    /// shared: one set of model entries serves every user; isolated: each
    /// user brings their own. Required.
    #[arg(long, value_enum)]
    mode: Option<InstanceMode>,
    /// Where the backend listens, which the proxy in front of it reaches.
    /// Required.
    #[arg(long, value_name = "ADDRESS:PORT")]
    listen: Option<SocketAddr>,
    /// Where uploads, media and the published programs are kept.
    #[arg(long, value_enum, default_value = "local")]
    storage: Storage,
    /// The S3 bucket, with --storage s3; the credentials come from
    /// AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.
    #[arg(long, value_name = "NAME")]
    s3_bucket: Option<String>,
    /// The bucket's region, with --storage s3.
    #[arg(long, value_name = "REGION")]
    s3_region: Option<String>,
    /// An S3-compatible service's HTTPS endpoint, with --storage s3.
    #[arg(long, value_name = "URL")]
    s3_endpoint: Option<String>,
    /// Names the bucket in the request path, with --storage s3.
    #[arg(long)]
    s3_force_path_style: bool,
    /// The domain of expose hostnames, which the proxy serves as
    /// *.<domain> with a wildcard certificate. Without it, exposes are off.
    /// Behind Cloudflare it is a zone of its own: its free certificate covers
    /// one level of wildcard.
    #[arg(long, value_name = "NAME")]
    expose_domain: Option<String>,
    /// The machine manager's state directory, on one filesystem.
    #[arg(long, value_name = "DIRECTORY", default_value = "/var/lib/demi/machine-manager")]
    cloud_data: PathBuf,
    /// Whether Clouds run under cgroup CPU, memory and PID limits.
    #[arg(long, value_enum, default_value = "on")]
    cloud_limits: Switch,
    /// The release to set up [default: this demi-server's].
    #[arg(long)]
    version: Option<Version>,
    /// A directory holding the release's assets instead of the GitHub
    /// releases.
    #[arg(long, value_name = "DIRECTORY", hide = true)]
    from: Option<PathBuf>,
    /// Install no system package: e2fsprogs, bsdtar and nftables are
    /// installed already.
    #[arg(long)]
    no_packages: bool,
    /// Never ask, as without a terminal.
    #[arg(long)]
    no_input: bool,
}

/// The backend's data directory on every server.
const BACKEND_DATA: &str = "/var/lib/demi/backend";

/// What setup reached, kept while a run has not finished: a `current`
/// without it is a server that is set up.
fn marker(layout: &Layout) -> PathBuf {
    layout.state().join("setup")
}

/// The choices that make the configuration.
struct Choices {
    domain: String,
    mode: InstanceMode,
    listen: SocketAddr,
}

/// How a run ended without an error.
#[derive(Debug)]
pub enum Outcome {
    SetUp,
    /// A required choice was missing and nothing could ask for it: the
    /// guide is the answer.
    Guide,
}

pub fn run(layout: &Layout, services: &dyn Services, mut options: Options) -> Result<Outcome, Box<dyn std::error::Error>> {
    let continuing = marker(layout).exists();
    if layout.current().exists() && !continuing {
        return Err("this machine is a Demi server already: `demi-server upgrade` moves it to a later release".into());
    }
    // Asked for nothing: the guide, whoever runs it.
    let asking = io::stdin().is_terminal() && !options.no_input;
    let Some(choices) = choices(&mut options, asking)? else {
        return Ok(Outcome::Guide);
    };
    crate::require_root(layout)?;
    let _lock = crate::lock(layout)?;
    let version = options.version.clone().unwrap_or_else(crate::own_version);
    let settings = configuration(&choices, &options)?;

    step("Checking the machine");
    let manager = machine()?;
    if !options.no_packages {
        step("Installing e2fsprogs, bsdtar and nftables");
        manager.install()?;
    }
    require_tools()?;
    if !continuing {
        // A backend this setup started already listens on a run that
        // continues.
        std::net::TcpListener::bind(choices.listen)
            .map_err(|error| format!("{} is not free to listen on: {error}", choices.listen))?;
    }

    step(&format!("Fetching Demi {version}"));
    let source = match &options.from {
        Some(directory) => fetch::Source::Directory(std::path::absolute(directory)?),
        None => fetch::Source::GitHub,
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(fetch::fetch(layout, &source, &version, &CancellationToken::new()))?;

    step("Checking that this machine can run Cloud");
    std::fs::create_dir_all(&options.cloud_data)?;
    let release = layout.release(&version);
    let checked = settings
        .command(Path::new("unshare"))
        .args(["--mount", "--propagation", "private"])
        .arg(release.join("bin/demi-machine-manager"))
        .arg("--check-host")
        .status()?;
    if !checked.success() {
        return Err("this machine cannot run Cloud, so Demi cannot be set up on it (the check above names why)".into());
    }

    step("Writing the installation");
    std::fs::create_dir_all(layout.state())?;
    std::fs::write(marker(layout), version.to_string())?;
    write_installation(layout, &settings)?;

    step("Starting Demi");
    layout.point_at(&version)?;
    layout.install_units(&version)?;
    services.reload()?;
    for unit in UNITS {
        services.start(unit).map_err(|error| format!("{unit} did not start: {error}\n{}", services.log(unit)))?;
    }
    link_program(layout)?;

    step(&format!("Checking https://{} from outside", choices.domain));
    runtime.block_on(check_outside(&choices, options.expose_domain.as_deref()))?;
    std::fs::remove_file(marker(layout))?;
    println!(
        "\nDemi {version} is set up at https://{0}.\n\
         Open https://{0} now and create the master account: until it exists, the first visitor creates it.",
        choices.domain
    );
    Ok(Outcome::SetUp)
}

fn step(name: &str) {
    println!("==> {name}");
}

/// The required choices: given, asked for at a terminal, or none, which
/// means the guide.
fn choices(options: &mut Options, asking: bool) -> io::Result<Option<Choices>> {
    if asking {
        if options.domain.is_none() {
            options.domain = Some(ask("Domain Demi is reached at over HTTPS", None)?);
        }
        if options.mode.is_none() {
            let mode = ask("Instance mode, shared or isolated", None)?;
            options.mode = Some(match mode.as_str() {
                "shared" => InstanceMode::Shared,
                "isolated" => InstanceMode::Isolated,
                other => return Err(io::Error::other(format!("{other} is no instance mode"))),
            });
        }
        if options.listen.is_none() {
            let listen = ask("Address and port the backend listens on", Some("127.0.0.1:3271"))?;
            options.listen = Some(listen.parse().map_err(io::Error::other)?);
        }
        if options.expose_domain.is_none() {
            let expose = ask("Domain of expose hostnames (empty for none)", Some(""))?;
            options.expose_domain = (!expose.is_empty()).then_some(expose);
        }
    }
    let (Some(domain), Some(mode), Some(listen)) = (options.domain.clone(), options.mode, options.listen) else {
        return Ok(None);
    };
    Ok(Some(Choices { domain, mode, listen }))
}

/// Asks at the terminal, offering `default`.
fn ask(question: &str, default: Option<&str>) -> io::Result<String> {
    match default {
        Some(default) if !default.is_empty() => print!("{question} [{default}]: "),
        _ => print!("{question}: "),
    }
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    let answer = answer.trim().to_owned();
    if answer.is_empty() {
        return match default {
            Some(default) => Ok(default.to_owned()),
            None => Err(io::Error::other(format!("{question}: an answer is required"))),
        };
    }
    Ok(answer)
}

/// The configuration file's settings for `choices`.
fn configuration(choices: &Choices, options: &Options) -> Result<Settings, Box<dyn std::error::Error>> {
    let domain = &choices.domain;
    let url = url::Url::parse(&format!("https://{domain}/"))?;
    if !matches!(url.host(), Some(url::Host::Domain(_))) || url.port().is_some() || url.path() != "/" {
        return Err(format!("{domain} is no domain: the public URL names a domain, never an address").into());
    }
    let mode = match choices.mode {
        InstanceMode::Shared => "shared",
        InstanceMode::Isolated => "isolated",
    };
    let mut variables = vec![
        ("DEMI_BACKEND_PUBLIC_URL".into(), format!("https://{domain}")),
        ("DEMI_INSTANCE_MODE".into(), mode.into()),
        ("DEMI_BACKEND_LISTEN".into(), choices.listen.to_string()),
        ("DEMI_BACKEND_DATA".into(), BACKEND_DATA.into()),
        ("DEMI_MANAGED_DATA".into(), options.cloud_data.display().to_string()),
    ];
    if options.cloud_limits == Switch::Off {
        variables.push(("DEMI_MANAGED_LIMITS".into(), "off".into()));
    }
    if let Some(expose) = &options.expose_domain {
        variables.push(("DEMI_EXPOSE_DOMAIN".into(), expose.clone()));
    }
    if options.storage == Storage::S3 {
        variables.push(("DEMI_STORAGE".into(), "s3".into()));
        let bucket = options.s3_bucket.clone().ok_or("--storage s3 needs --s3-bucket")?;
        let region = options.s3_region.clone().ok_or("--storage s3 needs --s3-region")?;
        variables.push(("DEMI_S3_BUCKET".into(), bucket));
        variables.push(("DEMI_S3_REGION".into(), region));
        if let Some(endpoint) = &options.s3_endpoint {
            variables.push(("DEMI_S3_ENDPOINT".into(), endpoint.clone()));
        }
        if options.s3_force_path_style {
            variables.push(("DEMI_S3_FORCE_PATH_STYLE".into(), "true".into()));
        }
        for name in ["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"] {
            let value = std::env::var(name).map_err(|_| format!("--storage s3 needs {name} in setup's environment"))?;
            variables.push((name.into(), value));
        }
    }
    Ok(Settings::new(variables))
}

/// A package manager of the distributions `setup` offers
/// (`installation.md` § Distributions).
enum PackageManager {
    Apt,
    Dnf,
    Pacman,
}

/// Requires systemd and a distribution `setup` offers.
fn machine() -> Result<PackageManager, Box<dyn std::error::Error>> {
    if !Path::new("/run/systemd/system").exists() {
        return Err("Demi runs its services under systemd, which is not this machine's init".into());
    }
    let release = std::fs::read_to_string("/etc/os-release")?;
    let field = |name: &str| {
        release
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(|value| value.trim_matches('"').to_owned())
            .unwrap_or_default()
    };
    let family = format!("{} {}", field("ID"), field("ID_LIKE"));
    let has = |name: &str| family.split_whitespace().any(|id| id == name);
    if has("debian") || has("ubuntu") {
        return Ok(PackageManager::Apt);
    }
    if has("fedora") || has("rhel") {
        return Ok(PackageManager::Dnf);
    }
    if has("arch") {
        return Ok(PackageManager::Pacman);
    }
    Err(format!(
        "{} is no distribution setup installs packages on: install e2fsprogs, bsdtar and nftables, then run setup again with --no-packages",
        field("PRETTY_NAME")
    )
    .into())
}

impl PackageManager {
    fn install(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut command = match self {
            Self::Apt => {
                let mut command = Command::new("apt-get");
                command
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .args(["install", "-y", "-q", "e2fsprogs", "libarchive-tools", "nftables"]);
                command
            }
            Self::Dnf => {
                let mut command = Command::new("dnf");
                command.args(["install", "-y", "-q", "e2fsprogs", "bsdtar", "nftables"]);
                command
            }
            Self::Pacman => {
                let mut command = Command::new("pacman");
                command.args(["-S", "--needed", "--noconfirm", "e2fsprogs", "libarchive", "nftables"]);
                command
            }
        };
        let status = command.status()?;
        if !status.success() {
            return Err(format!("installing the packages failed: {status}").into());
        }
        Ok(())
    }
}

/// Requires the programs the machine manager runs.
fn require_tools() -> Result<(), Box<dyn std::error::Error>> {
    let missing: Vec<&str> = ["mke2fs", "e2fsck", "resize2fs", "bsdtar", "nft"]
        .into_iter()
        .filter(|tool| which::which(tool).is_err())
        .collect();
    if !missing.is_empty() {
        return Err(format!("this machine lacks {}", missing.join(", ")).into());
    }
    Ok(())
}

/// The user and group the backend runs as, its data directory, the
/// manager's, and the configuration file, which a run that continues must
/// find as it wrote it.
fn write_installation(layout: &Layout, settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    let config = layout.config();
    let text = settings.file();
    match std::fs::read_to_string(&config) {
        Ok(existing) if existing == text => {}
        Ok(_) => {
            return Err(format!(
                "{} exists and says something else: setup never edits a configuration it did not write",
                config.display()
            )
            .into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir_all(config.parent().expect("the configuration lies in a directory"))?;
            demi_shared_artifacts::publish_bytes_blocking(
                &config,
                text.as_bytes(),
                Publication {
                    mode: Publish::CreateNew,
                    permissions: Permissions::Private,
                    durable: true,
                },
            )?;
        }
        Err(error) => return Err(error.into()),
    }
    if !succeeded(Command::new("getent").args(["group", "demi-cloud"]))? {
        checked(Command::new("groupadd").args(["--system", "demi-cloud"]))?;
    }
    if !succeeded(Command::new("id").arg("demi"))? {
        checked(Command::new("useradd").args([
            "--system",
            "--user-group",
            "--groups",
            "demi-cloud",
            "--home-dir",
            "/var/lib/demi",
            "--shell",
            "/usr/sbin/nologin",
            "demi",
        ]))?;
    }
    checked(Command::new("install").args(["-d", "-o", "root", "-g", "root", "-m", "0755", "/var/lib/demi"]))?;
    checked(Command::new("install").args(["-d", "-o", "demi", "-g", "demi", "-m", "0700", BACKEND_DATA]))?;
    let cloud = settings.manager_data()?;
    checked(Command::new("install").args(["-d", "-o", "root", "-g", "root", "-m", "0700"]).arg(&cloud))?;
    Ok(())
}

/// Links `/usr/local/bin/demi-server` to the current release's.
fn link_program(layout: &Layout) -> io::Result<()> {
    let bin = layout.root().join("usr/local/bin");
    std::fs::create_dir_all(&bin)?;
    let staged = bin.join(".demi-server.new");
    match std::fs::remove_file(&staged) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    std::os::unix::fs::symlink(layout.current().join("bin/demi-server"), &staged)?;
    std::fs::rename(staged, bin.join("demi-server"))
}

/// The server from outside: its domain answers over HTTPS with a valid
/// certificate, the proxy passes `Origin`, and an expose hostname reaches the
/// backend.
async fn check_outside(choices: &Choices, expose: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let client = demi_shared_artifacts::client()?;
    let domain = &choices.domain;
    let proxy = format!(
        "The proxy in front of Demi must serve https://{domain} and forward to {}, passing Origin and Host unchanged and allowing WebSocket upgrades; set it up and run the same command again.",
        choices.listen
    );
    let login = |origin: &str| {
        client
            .post(format!("https://{domain}/api/auth/login"))
            .header("origin", origin)
            .header("content-type", "application/json")
            .body("{}")
            .send()
    };
    let own = login(&format!("https://{domain}"))
        .await
        .map_err(|error| format!("https://{domain} does not answer: {}\n{proxy}", error.without_url()))?;
    if own.status() != 400 {
        return Err(format!("https://{domain} answers {} where Demi answers 400\n{proxy}", own.status()).into());
    }
    let other = login("https://elsewhere.example").await.map_err(|error| error.without_url().to_string())?;
    if other.status() != 403 {
        return Err(format!(
            "a request from another site was not refused ({}): the proxy drops Origin\n{proxy}",
            other.status()
        )
        .into());
    }
    if let Some(expose) = expose {
        let label = format!("check-{}", uuid::Uuid::new_v4().simple());
        let answer = client
            .get(format!("https://{label}.{expose}/"))
            .send()
            .await
            .map_err(|error| {
                format!(
                    "https://{label}.{expose} does not answer: {}\nThe proxy must serve *.{expose} with a wildcard certificate and forward it to Demi too.",
                    error.without_url()
                )
            })?;
        if answer.status() != 404 {
            return Err(format!("https://{label}.{expose} answers {} where Demi answers 404 for an expose it does not know", answer.status()).into());
        }
    }
    Ok(())
}

/// Whether `command` succeeds.
fn succeeded(command: &mut Command) -> io::Result<bool> {
    Ok(command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status()?.success())
}

/// Runs `command`, which must succeed.
fn checked(command: &mut Command) -> Result<(), Box<dyn std::error::Error>> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("{} failed: {status}", command.get_program().to_string_lossy()).into());
    }
    Ok(())
}
