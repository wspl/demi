//! A unit file as the supervisor reads it: the part of systemd's format the
//! release's own units use (`systemd/`). A directive the supervisor does not
//! carry out is refused, never passed over, so a release whose units ask for
//! more fails its first start in a container rather than run a service
//! otherwise than systemd would. The supervisor is always the release's own,
//! so its units and their reader change together.

use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

/// How a service tells that it started.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// It has started once its process runs.
    Simple,
    /// It says `READY=1` on its `NOTIFY_SOCKET`.
    Notify,
    /// It has done its work once its process ended well.
    Oneshot,
}

/// A `.service` unit.
pub struct Service {
    pub kind: Kind,
    pub exec_start: Vec<String>,
    pub exec_stop_post: Option<Vec<String>>,
    pub user: Option<String>,
    pub group: Option<String>,
    pub supplementary_groups: Vec<String>,
    pub umask: u32,
    /// `KillMode=process`: a stop signals the main process alone, and what
    /// it started runs on.
    pub kill_process_only: bool,
    pub restart_on_failure: bool,
    pub restart_delay: Duration,
    pub runtime_directories: Vec<String>,
    pub runtime_directory_mode: u32,
    pub runtime_directory_preserve: bool,
    pub environment_files: Vec<PathBuf>,
    pub environment: Vec<(String, String)>,
    pub private_mounts: bool,
    /// The capabilities the service may ever have, all when none.
    pub capability_bounding_set: Option<Vec<String>>,
    pub ambient_capabilities: Vec<String>,
}

/// A `.path` unit: it starts `unit` while `exists` exists.
pub struct PathUnit {
    pub exists: PathBuf,
    pub unit: String,
}

/// The directives that ask the supervisor for nothing: the ordering, which
/// its fixed order of the services gives; the installation, since it runs
/// every service it has; and a timeout of `infinity`, the only one it keeps.
fn asks_nothing(section: &str, key: &str, value: &str) -> bool {
    match (section, key) {
        ("Unit", "Description" | "After" | "Wants") | ("Install", "WantedBy") => true,
        ("Service", "TimeoutStartSec" | "TimeoutStopSec") => value == "infinity",
        _ => false,
    }
}

pub fn read_service(path: &Path) -> io::Result<Service> {
    let mut service = Service {
        kind: Kind::Simple,
        exec_start: Vec::new(),
        exec_stop_post: None,
        user: None,
        group: None,
        supplementary_groups: Vec::new(),
        umask: 0o022,
        kill_process_only: false,
        restart_on_failure: false,
        restart_delay: Duration::from_millis(100),
        runtime_directories: Vec::new(),
        runtime_directory_mode: 0o755,
        runtime_directory_preserve: false,
        environment_files: Vec::new(),
        environment: Vec::new(),
        private_mounts: false,
        capability_bounding_set: None,
        ambient_capabilities: Vec::new(),
    };
    for (section, key, value) in directives(path)? {
        let refused = || refusal(path, &key, &value);
        let words = || value.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        match (section.as_str(), key.as_str(), value.as_str()) {
            (section, key, value) if asks_nothing(section, key, value) => {}
            ("Service", "Type", "simple") => service.kind = Kind::Simple,
            ("Service", "Type", "notify") => service.kind = Kind::Notify,
            ("Service", "Type", "oneshot") => service.kind = Kind::Oneshot,
            ("Service", "ExecStart", _) => service.exec_start = command(&value).ok_or_else(refused)?,
            ("Service", "ExecStopPost", _) => service.exec_stop_post = Some(command(&value).ok_or_else(refused)?),
            ("Service", "User", _) => service.user = Some(value.clone()),
            ("Service", "Group", _) => service.group = Some(value.clone()),
            ("Service", "SupplementaryGroups", _) => service.supplementary_groups.extend(words()),
            ("Service", "UMask", _) => service.umask = octal(&value).ok_or_else(refused)?,
            ("Service", "KillMode", "process") => service.kill_process_only = true,
            ("Service", "KillMode", "control-group") => service.kill_process_only = false,
            ("Service", "Restart", "on-failure") => service.restart_on_failure = true,
            ("Service", "Restart", "no") => service.restart_on_failure = false,
            ("Service", "RestartSec", _) => {
                let seconds = value.strip_suffix('s').unwrap_or(&value);
                service.restart_delay = Duration::from_secs(seconds.parse().map_err(|_| refused())?);
            }
            ("Service", "RuntimeDirectory", _) => service.runtime_directories.extend(words()),
            ("Service", "RuntimeDirectoryMode", _) => service.runtime_directory_mode = octal(&value).ok_or_else(refused)?,
            ("Service", "RuntimeDirectoryPreserve", "yes") => service.runtime_directory_preserve = true,
            ("Service", "RuntimeDirectoryPreserve", "no") => service.runtime_directory_preserve = false,
            ("Service", "EnvironmentFile", _) if value.starts_with('/') => service.environment_files.push(PathBuf::from(&value)),
            ("Service", "Environment", _) => {
                for word in value.split_whitespace() {
                    let (name, setting) = word.split_once('=').filter(|_| !word.contains(['"', '\''])).ok_or_else(refused)?;
                    service.environment.push((name.to_owned(), setting.to_owned()));
                }
            }
            ("Service", "PrivateMounts", "yes") => service.private_mounts = true,
            ("Service", "PrivateMounts", "no") => service.private_mounts = false,
            ("Service", "CapabilityBoundingSet", _) => service.capability_bounding_set = Some(words()),
            ("Service", "AmbientCapabilities", _) => service.ambient_capabilities.extend(words()),
            _ => return Err(refused()),
        }
    }
    if service.exec_start.is_empty() {
        return Err(io::Error::other(format!("{} has no ExecStart", path.display())));
    }
    Ok(service)
}

/// The path unit at `path`, which starts the service of its own name unless
/// it names another.
pub fn read_path(path: &Path) -> io::Result<PathUnit> {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let mut exists = None;
    let mut unit = format!("{stem}.service");
    for (section, key, value) in directives(path)? {
        match (section.as_str(), key.as_str()) {
            _ if asks_nothing(&section, &key, &value) => {}
            ("Path", "PathExists") if value.starts_with('/') => exists = Some(PathBuf::from(&value)),
            ("Path", "Unit") => unit = value,
            _ => return Err(refusal(path, &key, &value)),
        }
    }
    let exists = exists.ok_or_else(|| io::Error::other(format!("{} has no PathExists", path.display())))?;
    Ok(PathUnit { exists, unit })
}

fn refusal(path: &Path, key: &str, value: &str) -> io::Error {
    io::Error::other(format!("{}: the supervisor does not carry out {key}={value}", path.display()))
}

/// Each directive as its section, its key and its value, in the file's
/// order.
fn directives(path: &Path) -> io::Result<Vec<(String, String, String)>> {
    let text = std::fs::read_to_string(path)?;
    let mut section = String::new();
    let mut directives = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            section = name.to_owned();
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .filter(|(_, value)| !value.ends_with('\\'))
            .ok_or_else(|| io::Error::other(format!("{}: the supervisor reads no line {line}", path.display())))?;
        directives.push((section.clone(), key.trim().to_owned(), value.trim().to_owned()));
    }
    Ok(directives)
}

/// A command line of plain words: no prefix such as `-` or `+`, no quoting
/// and no variable, which the units do not use.
fn command(value: &str) -> Option<Vec<String>> {
    let words: Vec<String> = value.split_whitespace().map(str::to_owned).collect();
    let plain = words.first().is_some_and(|program| program.starts_with('/'))
        && !value.contains(['"', '\'', '$', '%', '\\']);
    plain.then_some(words)
}

fn octal(value: &str) -> Option<u32> {
    u32::from_str_radix(value, 8).ok()
}
