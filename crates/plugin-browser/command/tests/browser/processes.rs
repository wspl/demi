#![cfg(unix)]
//! The process table as the tests read it, independently of the browser's
//! own process handling; the `browser` and `browser-processes` binaries
//! share it.

use std::{collections::HashSet, path::PathBuf};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

pub struct Process {
    pub pid: i32,
    pub parent: i32,
    pub group: i32,
}

/// Snapshot OS process relationships independently of the browser's cleanup
/// implementation: every process, without its threads, from the process
/// table as sysinfo reads it.
pub fn processes() -> Vec<Process> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    system
        .processes()
        .values()
        .filter_map(|process| {
            let pid = i32::try_from(process.pid().as_u32()).unwrap();
            // sysinfo does not report a process's group; the system call
            // does, and fails only for a process that has ended since.
            let group = unsafe { libc::getpgid(pid) };
            (group != -1).then(|| Process {
                pid,
                parent: process
                    .parent()
                    .map_or(0, |parent| i32::try_from(parent.as_u32()).unwrap()),
                group,
            })
        })
        .collect()
}

/// The profiles in `base` of the Chrome processes `parent` started, by main
/// process, read from the profiles' singleton locks rather than asked of the
/// service.
pub fn chrome_profiles_of(
    parent: u32,
    base: &std::path::Path,
) -> std::collections::BTreeMap<i32, PathBuf> {
    let parent = i32::try_from(parent).unwrap();
    let children: HashSet<_> = processes()
        .into_iter()
        .filter(|process| process.parent == parent)
        .map(|process| process.pid)
        .collect();
    let mut profiles = std::collections::BTreeMap::new();
    // Chrome can replace its Linux argv with a single process-title string.
    // Its singleton lock records the profile owner without parsing that title.
    for entry in std::fs::read_dir(base).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("demi-profile-")
        {
            continue;
        }
        let lock = match std::fs::read_link(path.join("SingletonLock")) {
            Ok(lock) => lock,
            // A profile that has not started, or has retired, owns no browser.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            // Another user's: on Unix every user's profiles share the base.
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
            Err(error) => panic!("read Chrome profile owner: {error}"),
        };
        let owner: i32 = lock
            .to_str()
            .unwrap()
            .rsplit_once('-')
            .unwrap()
            .1
            .parse()
            .unwrap();
        if children.contains(&owner) {
            profiles.insert(owner, path);
        }
    }
    profiles
}
