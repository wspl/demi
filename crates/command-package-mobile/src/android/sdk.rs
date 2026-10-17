//! The Android SDK of the Host (`mobile.md` § Android): its AVDs, as their
//! files describe them, the emulators that run, as their discovery files
//! say, and the emulator program that boots an AVD.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use futures_util::future::BoxFuture;

use super::{
    Avd, Running, Sdk,
    adb::Adb,
    emulator::Discovery,
    guest::{Guest, U2},
    process::Process,
};
use crate::driver::{DriverError, Result};

/// How long an emulator takes to say where it listens once started.
const ADVERTISING: Duration = Duration::from_secs(120);

/// The SDK where `ANDROID_HOME`, `ANDROID_SDK_ROOT` or the platform's
/// default place has it.
pub struct Installed {
    sdk: Option<PathBuf>,
    adb: Adb,
    /// Where an emulator Demi starts writes what it prints.
    logs: Option<PathBuf>,
    /// Starting the adb server, once.
    adb_started: tokio::sync::OnceCell<()>,
}

impl Installed {
    pub fn new(logs: Option<PathBuf>) -> Self {
        Self::at(sdk_root(), logs)
    }

    /// The SDK in `sdk`, or none.
    pub fn at(sdk: Option<PathBuf>, logs: Option<PathBuf>) -> Self {
        Self {
            sdk,
            adb: Adb::from_environment(),
            logs,
            adb_started: tokio::sync::OnceCell::new(),
        }
    }

    fn emulator(&self) -> Option<PathBuf> {
        let sdk = self.sdk.as_ref()?;
        let emulator = sdk.join("emulator").join(if cfg!(windows) { "emulator.exe" } else { "emulator" });
        emulator.is_file().then_some(emulator)
    }

    /// The adb server, started with the SDK's `adb` when it does not run.
    async fn adb(&self) -> Result<Adb> {
        self.adb_started
            .get_or_try_init(|| async {
                if self.adb.running().await {
                    return Ok(());
                }
                let adb = self
                    .sdk
                    .as_ref()
                    .map(|sdk| sdk.join("platform-tools").join(if cfg!(windows) { "adb.exe" } else { "adb" }))
                    .filter(|adb| adb.is_file())
                    .ok_or_else(|| {
                        DriverError::Failed(
                            "no adb; install the Android SDK's platform tools (sdkmanager \"platform-tools\")".into(),
                        )
                    })?;
                let status = tokio::process::Command::new(adb)
                    .arg("start-server")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .map_err(|error| DriverError::Failed(format!("adb did not start: {error}")))?;
                if !status.success() {
                    return Err(DriverError::Failed(format!("adb start-server ended with {status}")));
                }
                Ok(())
            })
            .await?;
        Ok(self.adb)
    }
}

/// The SDK's directory, where the environment or the platform's default
/// place has one.
fn sdk_root() -> Option<PathBuf> {
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(path) = std::env::var_os(variable).map(PathBuf::from)
            && path.is_dir()
        {
            return Some(path);
        }
    }
    let home = home()?;
    let default = if cfg!(target_os = "macos") {
        home.join("Library/Android/sdk")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?.join("Android").join("Sdk")
    } else {
        home.join("Android/Sdk")
    };
    default.is_dir().then_some(default)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Where the AVDs are, as the emulator looks for them.
fn avd_home() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ANDROID_AVD_HOME") {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("ANDROID_USER_HOME") {
        return Some(PathBuf::from(path).join("avd"));
    }
    Some(home()?.join(".android").join("avd"))
}

/// Where running emulators put their discovery files.
fn running_directory() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Caches/TemporaryItems")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?.join("Temp")
    } else {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(runtime) => PathBuf::from(runtime),
            None => {
                let user = std::env::var("USER").unwrap_or_default();
                std::env::temp_dir().join(format!("android-{user}"))
            }
        }
    };
    Some(base.join("avd").join("running"))
}

/// Watches `directory`: each change in it arrives at the receiver while
/// the watcher lives.
fn watch(directory: &Path) -> Result<(notify::RecommendedWatcher, tokio::sync::mpsc::UnboundedReceiver<()>)> {
    use notify::Watcher as _;
    let (changed, heard) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |_event| {
        // The waiting end is gone once what it waited for happened.
        let _ = changed.send(());
    })
    .map_err(|error| DriverError::Failed(format!("the running emulators cannot be watched: {error}")))?;
    watcher
        .watch(directory, notify::RecursiveMode::NonRecursive)
        .map_err(|error| DriverError::Failed(format!("{}: {error}", directory.display())))?;
    Ok((watcher, heard))
}

/// `key=value` lines.
fn values(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
}

fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    values(text).find(|(name, _)| *name == key).map(|(_, value)| value)
}

/// The AVD `name` whose `<name>.ini` holds `ini`, from its `config.ini`.
pub fn avd(name: &str, ini: &str, config: &str) -> Avd {
    let words = |id: &str| {
        id.split('_')
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut characters = word.chars();
                match characters.next() {
                    Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let api = value(ini, "target")
        .or_else(|| value(config, "image.sysdir.1").and_then(|sysdir| sysdir.split('/').find(|part| part.starts_with("android-"))))
        .and_then(|target| target.strip_prefix("android-"))
        .and_then(|level| level.split(['.', '-']).next())
        .and_then(|level| level.parse::<u32>().ok());
    Avd {
        name: name.to_owned(),
        display: value(config, "avd.ini.displayname")
            .map(str::to_owned)
            .unwrap_or_else(|| name.replace('_', " ")),
        model: value(config, "hw.device.name").map(words).unwrap_or_default(),
        os: api.map(android_version).unwrap_or_else(|| "Android".into()),
    }
}

/// The Android version of API level `api`.
fn android_version(api: u32) -> String {
    match api {
        36 => "Android 16".into(),
        35 => "Android 15".into(),
        34 => "Android 14".into(),
        33 => "Android 13".into(),
        32 => "Android 12L".into(),
        31 => "Android 12".into(),
        30 => "Android 11".into(),
        29 => "Android 10".into(),
        28 => "Android 9".into(),
        api => format!("Android API {api}"),
    }
}

/// The emulators the discovery files in `directory` describe whose
/// processes run, each with the time its process started.
fn discovered(directory: &Path) -> Vec<Running> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let pid: u32 = name.strip_prefix("pid_")?.strip_suffix(".ini")?.parse().ok()?;
            // A file whose emulator was killed stays behind.
            let process = system.process(sysinfo::Pid::from_u32(pid))?;
            let text = std::fs::read_to_string(entry.path()).ok()?;
            Some(Running {
                discovery: Discovery::parse(pid, &text)?,
                started: process.start_time(),
            })
        })
        .collect()
}

/// Each emulator process, qemu's, with the AVD its `-avd` names.
fn emulators() -> Vec<(String, Process)> {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
    );
    system
        .processes()
        .iter()
        .filter(|(_, process)| process.name().to_string_lossy().starts_with("qemu-system"))
        .filter_map(|(pid, process)| {
            let arguments = process.cmd();
            let at = arguments.iter().position(|argument| argument == "-avd")?;
            let avd = arguments.get(at + 1)?.to_str()?.to_owned();
            Some((
                avd,
                Process {
                    pid: pid.as_u32(),
                    started: process.start_time(),
                },
            ))
        })
        .collect()
}

impl Sdk for Installed {
    fn unavailable(&self) -> Option<String> {
        if self.sdk.is_none() {
            return Some(
                "no Android SDK; install Android Studio, or set ANDROID_HOME to the SDK's directory".into(),
            );
        }
        if self.emulator().is_none() {
            return Some("no Android emulator; install the Android SDK's emulator package (sdkmanager \"emulator\")".into());
        }
        None
    }

    fn avds(&self) -> BoxFuture<'_, Result<Vec<Avd>>> {
        Box::pin(async {
            let Some(home) = avd_home() else { return Ok(Vec::new()) };
            let Ok(entries) = std::fs::read_dir(&home) else { return Ok(Vec::new()) };
            let mut avds: Vec<Avd> = entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path.file_name()?.to_str()?.strip_suffix(".ini")?.to_owned();
                    let ini = std::fs::read_to_string(&path).ok()?;
                    let directory = value(&ini, "path")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| home.join(format!("{name}.avd")));
                    let config = std::fs::read_to_string(directory.join("config.ini")).unwrap_or_default();
                    Some(avd(&name, &ini, &config))
                })
                .collect();
            avds.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(avds)
        })
    }

    fn running(&self) -> BoxFuture<'_, Result<Vec<Running>>> {
        Box::pin(async {
            let Some(directory) = running_directory() else { return Ok(Vec::new()) };
            tokio::task::spawn_blocking(move || discovered(&directory))
                .await
                .map_err(|error| DriverError::Failed(format!("the running emulators were not read: {error}")))
        })
    }

    fn processes(&self) -> BoxFuture<'_, Vec<(String, Process)>> {
        Box::pin(async {
            tokio::task::spawn_blocking(emulators).await.unwrap_or_else(|error| {
                tracing::warn!("the emulators' processes were not read: {error}");
                Vec::new()
            })
        })
    }

    fn launch<'a>(&'a self, avd: &'a str) -> BoxFuture<'a, Result<Process>> {
        Box::pin(async move {
            let emulator = self
                .emulator()
                .ok_or_else(|| DriverError::Failed(self.unavailable().unwrap_or_default()))?;
            // An emulator of the AVD that is still ending, as right after a
            // close, holds the AVD until it ended.
            let ending: Vec<Running> =
                self.running().await?.into_iter().filter(|running| running.discovery.avd == avd).collect();
            for running in &ending {
                tokio::time::timeout(ADVERTISING, self.ended(running.process()))
                    .await
                    .map_err(|_| DriverError::Failed(format!("{avd} is still running")))?;
            }
            let (stdout, stderr) = match &self.logs {
                Some(logs) => {
                    let log = |error: std::io::Error| DriverError::Failed(format!("the emulator's log: {error}"));
                    std::fs::create_dir_all(logs).map_err(log)?;
                    let file = std::fs::File::create(logs.join(format!("{avd}.log"))).map_err(log)?;
                    (std::process::Stdio::from(file.try_clone().map_err(log)?), std::process::Stdio::from(file))
                }
                None => (std::process::Stdio::null(), std::process::Stdio::null()),
            };
            let mut command = tokio::process::Command::new(emulator);
            // Without a window, with the host's GPU, and with JWT
            // authentication, which keeps the gRPC interface on localhost.
            command
                .args(["-avd", avd, "-no-window", "-gpu", "host", "-no-boot-anim", "-grpc-use-jwt"])
                .stdin(std::process::Stdio::null())
                .stdout(stdout)
                .stderr(stderr);
            // The emulator outlives this program, which shuts nothing down
            // as it ends (`mobile.md` § Lifetime).
            #[cfg(unix)]
            command.process_group(0);
            #[cfg(windows)]
            command.creation_flags(0x0000_0200 | 0x0000_0008); // CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS
            let mut child = command
                .spawn()
                .map_err(|error| DriverError::Failed(format!("the emulator did not start: {error}")))?;
            // The launcher becomes the emulator itself, under its own id,
            // which the discovery file names.
            let process = child
                .id()
                .and_then(Process::of)
                .ok_or_else(|| DriverError::Failed("the emulator ended as it started".into()))?;
            // The process is waited for when it ends, so it leaves no zombie.
            tokio::spawn(async move {
                let _ended = child.wait().await;
            });
            Ok(process)
        })
    }

    fn advertised<'a>(&'a self, avd: &'a str, process: Process) -> BoxFuture<'a, Result<Running>> {
        Box::pin(async move {
            let directory = running_directory()
                .ok_or_else(|| DriverError::Failed("the emulators' running directory is unknown here".into()))?;
            std::fs::create_dir_all(&directory)
                .map_err(|error| DriverError::Failed(format!("{}: {error}", directory.display())))?;
            let (_watcher, mut heard) = watch(&directory)?;
            let advertised = async {
                loop {
                    let found = self.running().await?.into_iter().find(|running| running.discovery.pid == process.pid);
                    if let Some(found) = found {
                        return Ok(found);
                    }
                    tokio::select! {
                        heard = heard.recv() => if heard.is_none() {
                            return Err(DriverError::Failed("the running emulators cannot be watched".into()));
                        },
                        () = process.ended() => {
                            return Err(DriverError::Failed(format!(
                                "{avd}'s emulator ended as it started; its log is in the installation's mobile/android"
                            )));
                        }
                    }
                }
            };
            tokio::time::timeout(ADVERTISING, advertised)
                .await
                .map_err(|_| DriverError::Failed(format!("{avd} did not start in {} s", ADVERTISING.as_secs())))?
        })
    }

    fn ended(&self, process: Process) -> BoxFuture<'_, ()> {
        Box::pin(async move { process.ended().await })
    }

    fn terminate(&self, process: Process) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { process.end().await.map_err(DriverError::Failed) })
    }

    fn guest<'a>(&'a self, serial: &'a str) -> BoxFuture<'a, Result<Arc<dyn Guest>>> {
        Box::pin(async move {
            let adb = self.adb().await?;
            adb.wait_for(serial).await?;
            Ok(U2::new(adb, serial.to_owned()) as Arc<dyn Guest>)
        })
    }
}
