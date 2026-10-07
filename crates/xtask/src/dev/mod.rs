//! `xtask dev` (`backend.md` § One-command development backend): the
//! backend executable on a temporary data directory, or on one `--data`
//! names and keeps, with the backend
//! scenarios' scripted machine manager, whose runners run on this machine as
//! the Cloud, set up as the web app contract suite starts the backend
//! (`scenarios.md` § Web app contract suite), with a master account and the
//! development models `.env` turns on (a real one, the echo model) seeded
//! through the web API, and the preview domain service run locally under
//! `demi-preview.localhost` (`builds-and-releases.md` § Preview domain
//! deployment), where the backend registers its namespace with the web
//! development server's origins.

mod account;
mod echo;
mod provider;

use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use demi_command_protocol::testing::built_program;
use demi_runner_protocol::release::ServerRelease;
use demi_web_api_protocol::auth::SetupStatus;
use demi_web_api_protocol::providers::ProviderAnswer;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop, ProcessGroup};
use reqwest::header::{CONTENT_TYPE, COOKIE, SET_COOKIE};
use serde_json::json;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::signal::unix::SignalKind;
use tokio_util::sync::CancellationToken;

use crate::native::{self, Executable, development_package};
use account::Account;
use provider::DevProvider;

/// The one model of the echo entry.
const MODEL: &str = "echo";
/// The prefix of every program setting, which no program the command starts
/// may inherit from this one: each names its own settings, and the backend
/// and the runner refuse a variable that names none of them.
const SETTINGS: &[u8] = b"DEMI_";

/// How long the manager may take to print its socket, and the backend to
/// answer.
const START: Duration = Duration::from_secs(30);
/// How long the backend may take to stop: it hibernates the Cloud first.
const BACKEND_STOP: Duration = Duration::from_secs(10);
/// How long the manager may take to stop its runners.
const MANAGER_STOP: Duration = Duration::from_secs(5);
/// How long the preview domain service may take to answer: Wrangler starts
/// its local runtime first.
const PREVIEW_START: Duration = Duration::from_secs(60);
/// How long the preview domain service may take to stop.
const PREVIEW_STOP: Duration = Duration::from_secs(5);
/// How often the backend is asked whether it answers yet.
const POLL: Duration = Duration::from_millis(50);
/// How long one such question may wait for its answer.
const PROBE: Duration = Duration::from_secs(1);

#[derive(clap::Args)]
pub struct Options {
    /// The port the backend listens on.
    #[arg(long, default_value_t = 3271)]
    port: u16,
    /// Keep the data directory when the command ends.
    #[arg(long)]
    keep: bool,
    /// Run on this data directory, created when missing and kept when the
    /// command ends; a later run on it finds the account, the entries and
    /// the conversations of the earlier ones and seeds nothing.
    #[arg(long, conflicts_with = "keep")]
    data: Option<PathBuf>,
    /// The port the local preview domain service listens on, the preview
    /// domain being `demi-preview.localhost:<port>`.
    #[arg(long, default_value_t = 8787)]
    preview_port: u16,
    /// An origin the web app is served on, which the backend's preview
    /// namespace admits; repeat it for each.
    #[arg(long = "web-origin", default_value = "http://127.0.0.1:18934")]
    web_origins: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the machine manager {0}")]
    Manager(String),
    #[error("the preview domain service {0}")]
    Preview(String),
    #[error("the backend {0}")]
    Backend(String),
    #[error("{0}")]
    Seed(String),
    #[error("{0}")]
    DevProvider(String),
    #[error("{0}")]
    Account(String),
    #[error("a command program's development release: {0}")]
    Release(#[from] crate::native::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What the run seeds through the web API, read from the environment before
/// anything starts, so a partial setting stops the command at once.
struct Seed {
    account: Account,
    provider: Option<DevProvider>,
    /// Whether the echo model runs and has an entry.
    echo: bool,
}

pub fn run(options: Options) -> Result<(), Error> {
    let var = |name: &str| std::env::var(name).ok();
    let seed = Seed {
        account: Account::read(var).map_err(Error::Account)?,
        provider: DevProvider::read(var).map_err(Error::DevProvider)?,
        echo: echo::enabled(var).map_err(Error::Seed)?,
    };
    crate::interruptible(|cancel| develop(options, seed, cancel))?
}

/// Runs the development backend until `cancel` on the data directory
/// `--data` names, or on a temporary one it then removes unless `--keep`
/// keeps it, however the run ended.
async fn develop(
    options: Options,
    seed: Seed,
    cancel: CancellationToken,
) -> Result<(), Error> {
    if let Some(data) = &options.data {
        tokio::fs::create_dir_all(data).await?;
        let data = std::path::absolute(data)?;
        return serve(&options, &seed, &data, &cancel).await;
    }
    let root = tempfile::Builder::new().prefix("demi-dev-").tempdir()?;
    let served = serve(&options, &seed, root.path(), &cancel).await;
    let removed = if options.keep {
        println!("Kept the data directory {}", root.keep().display());
        Ok(())
    } else {
        root.close()
    };
    served?;
    removed?;
    Ok(())
}

/// The processes the command started, which `serve` stops in order.
#[derive(Default)]
struct Processes {
    backend: Option<Box<dyn ChildWrapper>>,
    manager: Option<Manager>,
    preview: Option<Box<dyn ChildWrapper>>,
}

/// The scripted manager and its input, which this program holds apart from
/// the process: waiting for a process closes the input it still holds, and
/// a closed input ends the manager.
struct Manager {
    process: Box<dyn ChildWrapper>,
    input: tokio::process::ChildStdin,
}

/// Starts the echo model when it is on, the manager and the backend under `root`, seeds
/// the account and the entries, and serves until `cancel` or until a process ends; then
/// stops the backend before the manager, since the backend hibernates the
/// Cloud through it as it shuts down.
async fn serve(
    options: &Options,
    seed: &Seed,
    root: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let echo = match seed.echo {
        true => Some(start_echo(options, root).await?),
        false => None,
    };
    let echo_url = echo.as_ref().map(|echo| echo.url.as_str());
    let mut processes = Processes::default();
    let result = tokio::select! {
        result = session(options, seed, root, echo_url, &mut processes) => result,
        () = cancel.cancelled() => Ok(()),
    };
    if let Some(backend) = processes.backend.take() {
        stop(backend, BACKEND_STOP, "the backend").await;
    }
    if let Some(manager) = processes.manager.take() {
        stop_manager(manager).await;
    }
    if let Some(preview) = processes.preview.take() {
        stop(preview, PREVIEW_STOP, "the preview domain service").await;
    }
    result
}

/// The echo model's endpoint. On a data directory `--data` names, it
/// listens on the port of the earlier runs, which the directory records,
/// since the entry seeded at the first run names that port.
async fn start_echo(options: &Options, root: &Path) -> Result<echo::Echo, Error> {
    if options.data.is_none() {
        return Ok(echo::start(0).await?);
    }
    let record = root.join("echo-port");
    let port = match tokio::fs::read_to_string(&record).await {
        Ok(text) => text
            .trim()
            .parse()
            .map_err(|error| Error::Seed(format!("{}: {error}", record.display())))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    let echo = echo::start(port).await?;
    tokio::fs::write(&record, echo.port.to_string()).await?;
    Ok(echo)
}

/// Everything between the first start and the end: it ends only with an
/// error, when a start or the seeding fails or a process ends by itself.
async fn session(
    options: &Options,
    seed: &Seed,
    root: &Path,
    echo: Option<&str>,
    processes: &mut Processes,
) -> Result<(), Error> {
    let preview = processes
        .preview
        .insert(spawn_preview(options.preview_port)?);
    let manager = processes.manager.insert(spawn_manager()?);
    let socket = manager_socket(manager.process.as_mut()).await?;

    let release = release_root(root).await?;
    let http = reqwest::Client::builder().no_proxy().build()?;
    // The backend registers its namespace as it starts.
    preview_answering(&http, options.preview_port, preview.as_mut()).await?;
    let origin = format!("http://127.0.0.1:{}", options.port);
    let preview_domain = format!("demi-preview.localhost:{}", options.preview_port);
    let backend = processes.backend.insert(spawn_backend(
        &root.join("backend"),
        options.port,
        &origin,
        &socket,
        &release,
        &preview_domain,
        &options.web_origins,
    )?);

    answering(&http, &origin, backend.as_mut()).await?;
    let models = match setup_needed(&http, &origin).await? {
        true => seed_data(&http, &origin, seed, echo).await?,
        // A data directory of an earlier run holds the account and the
        // entries it seeded.
        false => "\x20 Model:   the entries the data directory's first run seeded\n".to_owned(),
    };
    let kept = if options.data.is_some() || options.keep {
        " (kept)"
    } else {
        " (removed when the command ends)"
    };
    println!(
        "\nThe development backend serves at {origin}\n\
         \x20 Account: {}, password {}\n\
         {models}\
         \x20 Data:    {}{kept}\n\
         \x20 Preview: {preview_domain}, for the pages at {}\n\
         Start the page in another terminal:\n\
         \x20 DEMI_BACKEND_URL={origin} {}bun run web:dev\n\
         Ctrl-C stops the backend.\n",
        seed.account.email,
        if seed.account.from_env {
            "from DEMI_DEV_PASSWORD"
        } else {
            seed.account.password.as_str()
        },
        root.display(),
        options.web_origins.join(", "),
        page_account(&seed.account),
    );

    tokio::select! {
        status = backend.wait() => Err(Error::Backend(format!("exited: {}", status?))),
        status = manager.process.wait() => Err(Error::Manager(format!("exited: {}", status?))),
        status = preview.wait() => Err(Error::Preview(format!("exited: {}", status?))),
    }
}

/// Whether the backend still needs its master account: true on a new data
/// directory.
async fn setup_needed(http: &reqwest::Client, origin: &str) -> Result<bool, Error> {
    let answer = http.get(format!("{origin}/api/setup")).send().await?;
    let body = success(answer, "the setup status").await?.bytes().await?;
    let status: SetupStatus = serde_json::from_slice(&body)
        .map_err(|error| Error::Seed(format!("the setup status: {error}")))?;
    Ok(status.needed)
}

/// Seeds the master account and an entry for each development model;
/// answers the lines that name the models.
async fn seed_data(
    http: &reqwest::Client,
    origin: &str,
    seed: &Seed,
    echo: Option<&str>,
) -> Result<String, Error> {
    let cookies = setup(http, origin, &seed.account).await?;
    let mut models = String::new();
    if let Some(provider) = &seed.provider {
        let entry = create_entry(http, origin, &cookies, &provider.entry()).await?;
        let reads = match provider.accepted_extensions() {
            Some(types) => {
                let names: Vec<String> = types.iter().map(ToString::to_string).collect();
                format!("reads {} natively", names.join(", "))
            }
            None => "reads no file natively: its types are unknown".to_owned(),
        };
        models.push_str(&format!(
            "\x20 Model:   {} of the provider entry {entry}, a real model that calls tools\n\
             \x20          and {reads}\n",
            provider.model()
        ));
    }
    if let Some(echo) = echo {
        let entry = create_entry(http, origin, &cookies, &echo_entry(echo)).await?;
        models.push_str(&format!(
            "\x20 Model:   {MODEL} of the provider entry {entry}, which answers \"Echo: <your message>\"\n"
        ));
    }
    if models.is_empty() {
        models.push_str(
            "\x20 Model:   none; set DEMI_DEV_PROVIDER_* or DEMI_DEV_ECHO=1 in .env to add one\n",
        );
    }
    Ok(models)
}

/// Assembles under `root` the server release the backend publishes and
/// serves: a development release of each command program the build made,
/// for this machine's target, its descriptor in the root's `commands/` and
/// its compressed copy among the release's files, and nothing else, since
/// Vite serves the page and the scripted manager starts its runners itself
/// (`backend.md` § One-command development backend). Each run assembles it
/// anew from the programs built now. Answers the root.
async fn release_root(root: &Path) -> Result<PathBuf, Error> {
    // Never cancelled: an interrupt drops the whole session instead.
    let cancel = CancellationToken::new();
    let release = root.join("release");
    let files = root.join("files");
    let packaged = root.join("packages");
    for directory in [&release, &files, &packaged] {
        if tokio::fs::try_exists(directory).await? {
            tokio::fs::remove_dir_all(directory).await?;
        }
    }
    tokio::fs::create_dir_all(&release).await?;
    for command in Executable::COMMANDS {
        let name = command.name();
        let package = packaged.join(name);
        development_package(command, built_program(name), &package, &cancel).await?;
        native::split(&package, command, &release, &files, &cancel).await?;
    }
    let record = ServerRelease {
        files: files.to_string_lossy().into_owned(),
    };
    native::write_server_release(&release, &record).await?;
    Ok(release)
}

/// A command for the program `name` the workspace built beside this one.
fn program(name: &str) -> CommandWrap {
    let mut command = tokio::process::Command::new(built_program(name));
    command.current_dir(crate::repository());
    grouped(command)
}

/// `command` without this program's settings, in a process group of its
/// own, so that an interrupt at the terminal reaches only this program,
/// which stops the others in order; dropping it kills the group.
fn grouped(mut command: tokio::process::Command) -> CommandWrap {
    for (variable, _) in std::env::vars_os() {
        if variable.as_bytes().starts_with(SETTINGS) {
            command.env_remove(variable);
        }
    }
    let mut command = CommandWrap::from(command);
    command.wrap(ProcessGroup::leader()).wrap(KillOnDrop);
    command
}

/// Starts the preview domain service with Wrangler's development server on
/// `port`, under the domain `demi-preview.localhost:<port>`. Its namespaces
/// outlive the run in the package's `.wrangler/`, as the backend's record
/// does in a data directory `--data` names. Wrangler runs under Node, the
/// one runtime it supports: under Bun (1.4.2) its messages to its own local
/// proxy are dropped at random, and the service then never answers.
fn spawn_preview(port: u16) -> Result<Box<dyn ChildWrapper>, Error> {
    let node = system_node().ok_or_else(|| {
        Error::Preview("needs Node, and no Node that is not Bun is on PATH".to_owned())
    })?;
    let service = crate::repository().join("services/preview-domain");
    let mut command = tokio::process::Command::new(node);
    command
        .arg(service.join("node_modules/wrangler/bin/wrangler.js"))
        .args(["dev", "--port", &port.to_string(), "--var"])
        .arg(format!("PREVIEW_DOMAIN:demi-preview.localhost:{port}"))
        .current_dir(service);
    let mut command = grouped(command);
    command
        .command_mut()
        .env("WRANGLER_SEND_METRICS", "false")
        .stdin(Stdio::null());
    Ok(command.spawn()?)
}

/// The first `node` on PATH that is not Bun standing in for one, as Bun
/// puts its own first on the PATH of the scripts it runs.
fn system_node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("node"))
        .find(|candidate| {
            // A directory without a `node` offers none.
            std::fs::canonicalize(candidate)
                .is_ok_and(|real| real.file_name().is_some_and(|name| name != "bun"))
        })
}

/// Waits until the preview domain service answers on `port`, failing when
/// it exits first or does not answer within `PREVIEW_START`.
async fn preview_answering(
    http: &reqwest::Client,
    port: u16,
    preview: &mut dyn ChildWrapper,
) -> Result<(), Error> {
    // Any answer will do: the service answers its bare address with 404.
    let address = format!("http://127.0.0.1:{port}/");
    let probing = async {
        while http.get(&address).timeout(PROBE).send().await.is_err() {
            tokio::time::sleep(POLL).await;
        }
    };
    tokio::select! {
        () = probing => Ok(()),
        status = preview.wait() => Err(Error::Preview(format!(
            "exited before it answered: {}",
            status?
        ))),
        () = tokio::time::sleep(PREVIEW_START) => Err(Error::Preview(format!(
            "did not answer on port {port} within {} s",
            PREVIEW_START.as_secs()
        ))),
    }
}

/// Starts the scripted manager, the backend crate's example program
/// `scripted_machines`, whose Cloud runners keep their artifacts in
/// `.cache/dev-artifacts` in the repository. Its input stays open while
/// this program runs, and it ends when the input closes, however this
/// program ends.
fn spawn_manager() -> Result<Manager, Error> {
    let mut command = program("examples/scripted_machines");
    // The Cloud's runners share one cache that outlives every run.
    let artifacts = crate::repository().join(".cache/dev-artifacts");
    command
        .command_mut()
        .arg("--artifacts")
        .arg(artifacts)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut process = command.spawn()?;
    let input = process
        .stdin()
        .take()
        .expect("the manager's input is piped");
    Ok(Manager { process, input })
}

/// The socket the manager prints as its first line; the rest of its output
/// goes on to this program's.
async fn manager_socket(manager: &mut dyn ChildWrapper) -> Result<String, Error> {
    let output = manager
        .stdout()
        .take()
        .expect("the manager's output is piped");
    let mut output = BufReader::new(output);
    let mut line = String::new();
    let read = tokio::time::timeout(START, output.read_line(&mut line)).await;
    match read {
        Ok(Ok(read)) if read > 0 => {}
        Ok(Ok(_)) => return Err(Error::Manager("exited before it printed its socket".into())),
        Ok(Err(error)) => return Err(error.into()),
        Err(_) => {
            return Err(Error::Manager(format!(
                "printed no socket within {} s",
                START.as_secs()
            )));
        }
    }
    // The output ends with the manager, and the task with it.
    tokio::spawn(async move {
        let mut lines = output.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    Ok(line.trim_end().to_owned())
}

/// Starts the backend executable on the data directory `data`, with the
/// variables the web app contract suite gives it, in isolated mode.
fn spawn_backend(
    data: &Path,
    port: u16,
    origin: &str,
    socket: &str,
    release: &Path,
    preview_domain: &str,
    web_origins: &[String],
) -> Result<Box<dyn ChildWrapper>, Error> {
    let mut command = program("demi-backend");
    command
        .command_mut()
        .env("DEMI_RELEASE", release)
        .env("DEMI_BACKEND_DATA", data)
        .env("DEMI_BACKEND_LISTEN", format!("127.0.0.1:{port}"))
        .env("DEMI_INSTANCE_MODE", "isolated")
        .env("DEMI_BACKEND_PUBLIC_URL", origin)
        .env("DEMI_MACHINE_MANAGER_SOCKET", socket)
        .env("DEMI_PREVIEW_DOMAIN", preview_domain)
        .env("DEMI_PREVIEW_ORIGINS", web_origins.join(","))
        .stdin(Stdio::null());
    Ok(command.spawn()?)
}

/// Waits until the backend answers `GET /api/setup`, failing when it exits
/// first or does not answer within `START`. Each probe waits `PROBE` at
/// most: a port another program holds may take the request and never
/// answer it.
async fn answering(
    http: &reqwest::Client,
    origin: &str,
    backend: &mut dyn ChildWrapper,
) -> Result<(), Error> {
    let probing = async {
        loop {
            let probe = http.get(format!("{origin}/api/setup")).timeout(PROBE);
            if probe
                .send()
                .await
                .is_ok_and(|answer| answer.status().is_success())
            {
                return;
            }
            tokio::time::sleep(POLL).await;
        }
    };
    tokio::select! {
        () = probing => Ok(()),
        status = backend.wait() => Err(Error::Backend(format!(
            "exited before it answered: {}",
            status?
        ))),
        () = tokio::time::sleep(START) => Err(Error::Backend(format!(
            "did not answer within {} s",
            START.as_secs()
        ))),
    }
}

/// The account variables the page's command names: none when `.env` names
/// the account, since the page reads it from there.
fn page_account(account: &Account) -> String {
    if account.from_env {
        return String::new();
    }
    format!(
        "DEMI_DEV_EMAIL={} DEMI_DEV_PASSWORD={} ",
        account.email, account.password
    )
}

/// Creates the master account, which setup signs in; answers its session's
/// cookies.
async fn setup(http: &reqwest::Client, origin: &str, account: &Account) -> Result<String, Error> {
    let setup = json!({
        "nickname": "Developer",
        "email": account.email,
        "password": account.password,
    });
    let answer = http
        .post(format!("{origin}/api/setup"))
        .header(CONTENT_TYPE, "application/json")
        .body(setup.to_string())
        .send()
        .await?;
    let answer = success(answer, "setup").await?;
    Ok(session_cookies(&answer))
}

/// The echo model's provider entry, of the `anthropic` family with one
/// configured model.
fn echo_entry(echo: &str) -> serde_json::Value {
    json!({
        "source": "custom",
        "providerType": "anthropic",
        "label": "Echo",
        "apiKey": "sk-ant-echo",
        "baseUrl": echo,
        "models": [{
            "id": MODEL,
            "displayName": "Echo",
            "contextWindow": 200_000,
            "outputLimit": null,
            "thinkingEfforts": [],
            "acceptedExtensions": null,
            "fastTier": null,
        }],
    })
}

/// Creates the provider entry `entry` with the session's `cookies`; answers
/// its id. The entry's key reaches the backend only in this request.
async fn create_entry(
    http: &reqwest::Client,
    origin: &str,
    cookies: &str,
    entry: &serde_json::Value,
) -> Result<String, Error> {
    let answer = http
        .post(format!("{origin}/api/providers"))
        .header(CONTENT_TYPE, "application/json")
        .header(COOKIE, cookies)
        .body(entry.to_string())
        .send()
        .await?;
    let body = success(answer, "the provider entry").await?.bytes().await?;
    let answer: ProviderAnswer = serde_json::from_slice(&body)
        .map_err(|error| Error::Seed(format!("the provider entry's answer: {error}")))?;
    Ok(answer.provider.id.to_string())
}

/// `answer` when it succeeded; its status and body otherwise.
async fn success(answer: reqwest::Response, what: &str) -> Result<reqwest::Response, Error> {
    let status = answer.status();
    if status.is_success() {
        return Ok(answer);
    }
    let body = answer.text().await?;
    Err(Error::Seed(format!("{what} answered {status}: {body}")))
}

/// The cookies `answer` sets, as a `Cookie` header sends them back.
fn session_cookies(answer: &reqwest::Response) -> String {
    let pairs: Vec<&[u8]> = answer
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|cookie| cookie.as_bytes().split(|byte| *byte == b';').next())
        .collect();
    String::from_utf8_lossy(&pairs.join(&b"; "[..])).into_owned()
}

/// Asks `child` to stop and waits `patience` for it to end.
async fn stop(mut child: Box<dyn ChildWrapper>, patience: Duration, name: &str) {
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    if let Err(error) = child.signal(SignalKind::terminate().as_raw_value()) {
        eprintln!("xtask dev: {name} could not be asked to stop: {error}");
    }
    reap(child, patience, name).await;
}

/// Closes the manager's input, which ends it and the runners it started.
async fn stop_manager(manager: Manager) {
    drop(manager.input);
    reap(manager.process, MANAGER_STOP, "the machine manager").await;
}

/// Waits `patience` for `child` to end, then kills its process group.
async fn reap(mut child: Box<dyn ChildWrapper>, patience: Duration, name: &str) {
    let ended = tokio::time::timeout(patience, child.wait()).await;
    let killed = match ended {
        Ok(Ok(_)) => return,
        Ok(Err(error)) => {
            eprintln!("xtask dev: waiting for {name} failed: {error}");
            Box::into_pin(child.kill()).await
        }
        Err(_) => {
            eprintln!(
                "xtask dev: {name} did not stop within {} s and is killed",
                patience.as_secs()
            );
            Box::into_pin(child.kill()).await
        }
    };
    if let Err(error) = killed {
        eprintln!("xtask dev: {name} could not be killed: {error}");
    }
}
