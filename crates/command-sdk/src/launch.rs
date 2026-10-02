//! How a runner starts a command program, and how the program reads it
//! (`native-runtime.md` § Invoke and retire a service): `--command-service`,
//! then `--resource <name>=<path>` for each of its package's resources,
//! naming the resource's entry.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::{CommandFactory as _, Parser};

/// What a command program was started with.
#[derive(Debug, Parser)]
#[command(disable_help_flag = true, disable_version_flag = true)]
pub struct Launch {
    /// Serves the command protocol over standard input and output.
    #[arg(long = "command-service", required = true)]
    _command_service: bool,
    /// The entry of a resource of the program's package; repeat for each.
    #[arg(long = "resource", value_name = "NAME=PATH", value_parser = resource)]
    resources: Vec<(String, PathBuf)>,
}

impl Launch {
    /// The process's arguments; anything else prints the usage and exits
    /// with status 2.
    pub fn from_process() -> Self {
        let launch = Self::parse();
        let mut names = std::collections::HashSet::new();
        if let Some((name, _)) = launch
            .resources
            .iter()
            .find(|(name, _)| !names.insert(name))
        {
            Self::command()
                .error(
                    clap::error::ErrorKind::ArgumentConflict,
                    format!("the resource {name} is named twice"),
                )
                .exit();
        }
        launch
    }

    /// The entry of the resource `name`, when the runner gave it.
    pub fn resource(&self, name: &str) -> Option<&Path> {
        self.resources
            .iter()
            .find(|(given, _)| given == name)
            .map(|(_, path)| path.as_path())
    }
}

/// The arguments a runner starts a command program with, given the entry of
/// each of its package's resources by name.
pub fn launch_arguments(resources: &BTreeMap<String, PathBuf>) -> Vec<OsString> {
    let mut arguments = vec![OsString::from("--command-service")];
    for (name, entry) in resources {
        let mut resource = OsString::from(format!("{name}="));
        resource.push(entry);
        arguments.push(OsString::from("--resource"));
        arguments.push(resource);
    }
    arguments
}

fn resource(value: &str) -> Result<(String, PathBuf), String> {
    let (name, path) = value
        .split_once('=')
        .ok_or_else(|| format!("{value} is not NAME=PATH"))?;
    let path = PathBuf::from(path);
    if name.is_empty() || !path.is_absolute() {
        return Err(format!(
            "{value} does not name a resource and its absolute path"
        ));
    }
    Ok((name.to_owned(), path))
}
