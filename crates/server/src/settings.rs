//! The installation's configuration, `/opt/demi/config/demi.env`, as the two
//! services read it: an upgrade runs the next release's programs with it and
//! finds the data directories in it. Reading it never changes it.

use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Settings {
    variables: Vec<(String, String)>,
}

impl Settings {
    pub fn new(variables: Vec<(String, String)>) -> Self {
        Self { variables }
    }

    pub fn read(path: &Path) -> io::Result<Self> {
        let variables = dotenvy::from_path_iter(path)
            .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
        Ok(Self { variables })
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.variables
            .iter()
            .rev()
            .find(|(variable, _)| variable == name)
            .map(|(_, value)| value.as_str())
    }

    /// A data directory the configuration must name, since an upgrade copies
    /// from it or reads it.
    fn directory(&self, name: &str) -> io::Result<PathBuf> {
        let value = self
            .value(name)
            .ok_or_else(|| io::Error::other(format!("the configuration names no {name}")))?;
        Ok(PathBuf::from(value))
    }

    /// The backend's data directory.
    pub fn backend_data(&self) -> io::Result<PathBuf> {
        self.directory("DEMI_BACKEND_DATA")
    }

    /// The machine manager's state directory.
    pub fn manager_data(&self) -> io::Result<PathBuf> {
        self.directory("DEMI_MANAGED_DATA")
    }

    /// The configuration file's text: one `NAME="value"` per line, which
    /// systemd's environment files and `read` both read.
    pub fn file(&self) -> String {
        self.variables
            .iter()
            .map(|(name, value)| {
                let value = value.replace('\\', "\\\\").replace('"', "\\\"");
                format!("{name}=\"{value}\"\n")
            })
            .collect()
    }

    /// `program` with the configuration as its environment, as systemd
    /// starts a service with the file: nothing else of this process's
    /// environment but its `PATH`.
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        command.envs(self.variables.iter().map(|(name, value)| (name, value)));
        command
    }
}
