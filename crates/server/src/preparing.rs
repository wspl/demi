//! The work ahead of an update: after the backend's check finds a release
//! newer than the one the server runs, the server gets it ready while the
//! current release serves, so Update Now has only the checks and the restart
//! left.
//!
//! ```text
//! backend's check ─► prepare request ─► demi-prepare.service (root):
//!   installed demi-server prepare --requested   fetch: download, unpack
//!     └─► the release's own demi-server prepare  gVisor, image, devices
//! ```
//!
//! - It changes nothing that runs: the release unpacks beside the current
//!   one, the image imports beside the running manager's, and the devices
//!   download the runner beside the one they run.
//! - It keeps one release ready, never more: getting a newer one ready
//!   removes the release ready before it, with its gVisor version, and the
//!   new manager's import removes the Cloud base imported before it.
//! - Its steps reach the backend as notices of their own (`notices`), which
//!   the administrators' row shows as how far the release is got ready.
//! - It gives way to every other run of `demi-server`: one that holds the
//!   lock, such as an update, ends it at once, and the next check asks again.
//!   An update that starts while it runs waits for it, then finds its work
//!   done.

use std::io;

use semver::Version;

use demi_runner_protocol::release::{SkipReason, UpgradeStepKind};

use crate::{
    fetch, gvisor,
    journal::Journal,
    layout::Layout,
    moving,
    notices::Notices,
    runners,
    settings::Settings,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Gets the release the backend's request names ready: removes a release
/// ready before it, fetches it, and hands over to its own `prepare`.
pub fn requested(layout: &Layout) -> Result<()> {
    let Some(version) = crate::read_request(&layout.prepare_request())? else {
        return Ok(());
    };
    // An interrupted move is finished by the next update, not by this.
    if Journal::read(&layout.journal())?.is_some() {
        return Ok(());
    }
    let current = layout.current_version()?;
    if version <= current {
        return Ok(());
    }
    let mut notices = Notices::preparing(layout);
    notices.moving(&current, &version);
    let fetched = keep_one(layout, &current, &version)
        .map_err(Into::into)
        .and_then(|()| crate::fetch_telling(layout, &fetch::Source::GitHub, &version, &mut notices));
    if let Err(error) = fetched {
        notices.failed(error.to_string(), false);
        return Err(error);
    }
    // The release tells the backend from here on.
    drop(notices);
    Err(crate::hand_over(layout, &version, "prepare").into())
}

/// Gets this program's own release ready, which the installed `demi-server`
/// fetched: its gVisor version and its image, and the devices' runner.
pub fn own(layout: &Layout) -> Result<()> {
    let to = crate::own_version();
    let current = layout.current_version()?;
    if to <= current {
        return Ok(());
    }
    let mut notices = Notices::preparing(layout);
    notices.moving(&current, &to);
    for kind in [UpgradeStepKind::Download, UpgradeStepKind::Unpack] {
        notices.skip_waiting(kind, SkipReason::Installed);
    }
    let installed = Settings::read(&layout.config())
        .map_err(io::Error::other)
        .and_then(|settings| moving::install(layout, &settings, &to, &mut notices).map(|()| settings));
    let settings = match installed {
        Ok(settings) => settings,
        Err(error) => {
            notices.failed(error.to_string(), false);
            return Err(error.into());
        }
    };
    drop(notices);
    let socket = settings
        .manage_socket()
        .unwrap_or_else(|| layout.manage_socket());
    runners::announce(&socket, &layout.release(&to))?;
    println!("Demi {to} is ready to install");
    Ok(())
}

/// Removes every release newer than `current` but `version`, a release got
/// ready before it, with the gVisor versions no release kept uses.
fn keep_one(layout: &Layout, current: &Version, version: &Version) -> io::Result<()> {
    let mut kept = Vec::new();
    for each in layout.versions()? {
        if &each > current && &each != version {
            println!("Removing Demi {each}, which {version} replaces");
            std::fs::remove_dir_all(layout.release(&each))?;
        } else {
            kept.push(each);
        }
    }
    gvisor::prune(layout, &kept.iter().collect::<Vec<_>>())
}
