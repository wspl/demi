//! Environment operations confined to one utility invocation.

pub use std::env::{consts, join_paths, split_paths, JoinPathsError, VarError};
use std::{ffi::{OsStr, OsString}, path::{Path, PathBuf}};

pub type Vars = std::vec::IntoIter<(String, String)>;
pub type VarsOs = std::vec::IntoIter<(OsString, OsString)>;

pub fn var(key: impl AsRef<OsStr>) -> Result<String, VarError> {
    var_os(key).map(|value| value.into_string().map_err(VarError::NotUnicode)).unwrap_or(Err(VarError::NotPresent))
}
pub fn var_os(key: impl AsRef<OsStr>) -> Option<OsString> {
    let key = key.as_ref().to_str()?;
    super::CURRENT.with(|current| current.borrow().as_ref().expect("utility context").env.get(key).map(OsString::from))
}
pub fn vars() -> Vars {
    super::CURRENT.with(|current| current.borrow().as_ref().expect("utility context").env.clone().into_iter().collect::<Vec<_>>().into_iter())
}
pub fn vars_os() -> VarsOs {
    vars().map(|(key, value)| (key.into(), value.into())).collect::<Vec<_>>().into_iter()
}
/// Matches the standard-library signature, but changes only invocation state.
pub unsafe fn set_var(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    let key = key.as_ref().to_string_lossy().into_owned();
    let value = value.as_ref().to_string_lossy().into_owned();
    super::CURRENT.with(|current| { current.borrow_mut().as_mut().expect("utility context").env.insert(key, value); });
}
/// Matches the standard-library signature, but changes only invocation state.
pub unsafe fn remove_var(key: impl AsRef<OsStr>) {
    super::CURRENT.with(|current| { current.borrow_mut().as_mut().expect("utility context").env.remove(key.as_ref().to_string_lossy().as_ref()); });
}
pub fn current_dir() -> std::io::Result<PathBuf> {
    super::CURRENT.with(|current| Ok(current.borrow().as_ref().expect("utility context").cwd.clone()))
}
pub fn set_current_dir(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = std::fs::canonicalize(super::resolve(path))?;
    if !path.is_dir() { return Err(std::io::Error::new(std::io::ErrorKind::NotADirectory, "not a directory")); }
    super::CURRENT.with(|current| { current.borrow_mut().as_mut().expect("utility context").cwd = path; });
    Ok(())
}
pub fn temp_dir() -> PathBuf {
    var_os("TMPDIR").or_else(|| var_os("TEMP")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}
pub fn home_dir() -> Option<PathBuf> { var_os("HOME").or_else(|| var_os("USERPROFILE")).map(PathBuf::from) }
pub use std::env::{args, args_os, current_exe};
