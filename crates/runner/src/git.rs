//! Git for Windows, whose `usr\bin` holds the utilities a job on Windows runs
//! besides Demi's own (`crates/runner-shell/src/utilities.rs`): `ls`, `grep`,
//! `awk`, `tar` and the rest.
//!
//! The runner finds it, whoever installed it, by:
//!
//! 1. the `InstallPath` of `HKCU\Software\GitForWindows`, where an installation
//!    for one user records itself, then of `HKLM\SOFTWARE\GitForWindows`, where
//!    one for every user does;
//! 2. its usual folders, `%ProgramFiles%\Git` and `%LOCALAPPDATA%\Programs\Git`;
//! 3. the `git.exe` on `PATH`, which lies in `cmd`, `bin` or `mingw64\bin`
//!    below the installation.
//!
//! A folder counts when it has a `usr\bin`. The device's installer asks the
//! runner for it (`demi-runner git`) before it installs Git for Windows itself
//! (`crates/backend-runners/src/install.rs`).
//!
//! The device environment has the folder's `usr\bin` in `PATH`: where the Host's
//! `PATH` holds it already it stays there, and where not it is added at the end,
//! so Windows' own programs, such as `tar` or the `find` a batch file calls, keep
//! their names. It is found when the runner starts: one installed since is used
//! after the runner starts again.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
};

/// The folder Git for Windows is installed in, if the runner finds one.
pub fn find() -> Option<PathBuf> {
    let registered = [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .filter_map(install_path);
    let usual = [
        std::env::var_os("ProgramFiles").map(|folder| PathBuf::from(folder).join("Git")),
        std::env::var_os("LOCALAPPDATA").map(|folder| PathBuf::from(folder).join("Programs").join("Git")),
    ]
    .into_iter()
    .flatten();
    let listed = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .filter(|directory| directory.join("git.exe").is_file())
        .filter_map(|directory| directory.ancestors().find(|folder| has_utilities(folder)).map(Path::to_owned));
    registered
        .chain(usual)
        .chain(listed)
        .find(|folder| has_utilities(folder))
}

fn has_utilities(folder: &Path) -> bool {
    folder.join("usr").join("bin").is_dir()
}

/// The `InstallPath` Git for Windows' installer wrote under `root`.
fn install_path(root: HKEY) -> Option<PathBuf> {
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }
    let (key, value) = (wide(r"Software\GitForWindows"), wide("InstallPath"));
    let mut size = 0u32;
    // SAFETY: the key and the value are NUL-terminated UTF-16 strings that
    // outlive both calls; the first call only asks for the size in bytes, and
    // the second gets a buffer of exactly that size.
    unsafe {
        let status = RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size);
        if status != ERROR_SUCCESS || size == 0 {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        let status = RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size);
        if status != ERROR_SUCCESS {
            return None;
        }
        let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
        Some(PathBuf::from(<OsString as std::os::windows::ffi::OsStringExt>::from_wide(&buffer[..length])))
    }
}

/// Adds the `usr\bin` of the Git for Windows the runner finds at the end of
/// `env`'s `PATH`, unless the `PATH` has it already, wherever: Windows' own
/// programs, `tar` and the `find` a batch file calls among them, keep their
/// names. Without a Git, `env` is as it was.
pub fn complete(env: &mut BTreeMap<String, String>) {
    let Some(folder) = find() else { return };
    let utilities = folder.join("usr").join("bin");
    let utilities = utilities.to_string_lossy().into_owned();
    let path = env.get("PATH").cloned().unwrap_or_default();
    let present = path
        .split(';')
        .any(|entry| entry.trim_end_matches('\\').eq_ignore_ascii_case(&utilities));
    if present {
        return;
    }
    let path = if path.is_empty() { utilities } else { format!("{path};{utilities}") };
    env.insert("PATH".into(), path);
}
