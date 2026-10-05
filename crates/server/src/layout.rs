//! Where a server keeps its releases, its configuration, its units and the
//! state of an upgrade (`upgrades.md` § One release on a server). Every path
//! lies beneath a root, `/` on a server, so a test runs a whole upgrade in a
//! directory of its own.

use std::{
    io,
    path::{Path, PathBuf},
};

use semver::Version;

/// The services, in the order they start; they stop in the reverse order.
pub const UNITS: [&str; 2] = ["demi-machine-manager.service", "demi-backend.service"];

pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The directory every path lies beneath.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn at(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// Each release's root, by its version.
    pub fn releases(&self) -> PathBuf {
        self.at("/opt/demi/releases")
    }

    pub fn release(&self, version: &Version) -> PathBuf {
        self.releases().join(version.to_string())
    }

    /// The link to the release the services run.
    pub fn current(&self) -> PathBuf {
        self.at("/opt/demi/current")
    }

    /// The installation's configuration, which both services load.
    pub fn config(&self) -> PathBuf {
        self.at("/etc/demi/demi.env")
    }

    /// Where systemd reads the services' units.
    pub fn units(&self) -> PathBuf {
        self.at("/etc/systemd/system")
    }

    /// `demi-server`'s own state: its lock and an upgrade's journal.
    pub fn state(&self) -> PathBuf {
        self.at("/var/lib/demi/server")
    }

    pub fn journal(&self) -> PathBuf {
        self.state().join("upgrade.json")
    }

    /// The version of the release `current` names.
    pub fn current_version(&self) -> io::Result<Version> {
        let target = std::fs::read_link(self.current())?;
        version_of(&target)
    }

    /// Points `current` at `version`'s release, replacing the link in one
    /// rename.
    pub fn point_at(&self, version: &Version) -> io::Result<()> {
        let current = self.current();
        let staged = current.with_file_name(".current.new");
        match std::fs::remove_file(&staged) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        std::os::unix::fs::symlink(self.release(version), &staged)?;
        std::fs::rename(&staged, &current)?;
        sync_directory(current.parent().expect("the link lies in a directory"))
    }

    /// The releases unpacked under `releases/`, a stage aside.
    pub fn versions(&self) -> io::Result<Vec<Version>> {
        let mut versions = Vec::new();
        let entries = match std::fs::read_dir(self.releases()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(versions),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            versions.push(parse(&name)?);
        }
        versions.sort();
        Ok(versions)
    }

    /// Copies `version`'s units into place for systemd.
    pub fn install_units(&self, version: &Version) -> io::Result<()> {
        let units = self.units();
        std::fs::create_dir_all(&units)?;
        for unit in UNITS {
            let source = self.release(version).join("systemd").join(unit);
            let staged = units.join(format!(".{unit}.new"));
            std::fs::copy(&source, &staged)?;
            std::fs::rename(&staged, units.join(unit))?;
        }
        sync_directory(&units)
    }
}

/// The version a release's directory is named by.
fn version_of(path: &Path) -> io::Result<Version> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other(format!("{} names no release", path.display())))?;
    parse(&name.to_string_lossy())
}

fn parse(name: &str) -> io::Result<Version> {
    Version::parse(name)
        .map_err(|error| io::Error::other(format!("{name} is not a release's version: {error}")))
}

/// Syncs a directory's entries, so a rename in it survives a crash.
pub fn sync_directory(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}
