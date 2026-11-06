//! `demi-server container`, the entry point of Demi's container image: it
//! writes the configuration from the container's variables, decides what
//! the volume at `/opt/demi` needs, and becomes the supervisor of the
//! services (`supervise`).
//!
//! The image holds one release's assets, a local mirror of them ([`ASSETS`]),
//! and the volume holds the server, laid out as on any machine. So the image
//! is a seed, and **Settings › Updates** moves the server past it:
//!
//! ```text
//! the volume holds               first, before the services
//! a move that did not finish     the move's release finishes it
//! no server, or a setup that     the image's demi-server sets it up from
//!   did not finish                 the assets
//! a release older than the       the image's demi-server upgrades to its
//!   image's                        release from the assets
//! the image's or a newer one     nothing: the current release's supervisor
//!                                  starts the services
//! ```
//!
//! The image's own supervisor runs that first work, which starts the
//! services itself, and execs the current release's supervisor once a start
//! finds `current` naming another release.
//!
//! In a container the platform's variables are the configuration: each start
//! writes `config/demi.env` from its `DEMI_*` and `AWS_*` variables, and
//! nothing else writes it, so an operator changes a setting in the platform
//! and the next start takes it.
//!
//! A container's `/run` outlives a restart of the same container, where a
//! machine's is empty after each boot, so a start first removes what Demi's
//! programs left there.

use std::{os::unix::process::CommandExt as _, path::PathBuf};

use demi_shared_artifacts::{Mode as Publish, Permissions, Publication};

use crate::{journal::Journal, layout::Layout, settings::Settings};

/// The release's assets in the image, as `demi-server` fetches them.
pub const ASSETS: &str = "/usr/lib/demi/assets";

pub fn start(layout: &Layout) -> Result<(), Box<dyn std::error::Error>> {
    crate::require_root(layout)?;
    for entry in std::fs::read_dir(layout.run_directory())? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("demi-") {
            match entry.file_type()?.is_dir() {
                true => std::fs::remove_dir_all(entry.path())?,
                false => std::fs::remove_file(entry.path())?,
            }
        }
    }
    write_configuration(layout)?;
    if Settings::read(&layout.config())?.public_url().is_none() {
        return Err("DEMI_BACKEND_PUBLIC_URL is not set: set it among the container's variables to the \
                    address Demi is reached at, such as https://demi.example.com"
            .into());
    }
    let image = crate::own_version();
    let current = match layout.current().exists() {
        true => Some(layout.current_version()?),
        false => None,
    };
    // The container's units directory is the image's, empty at each start.
    if let Some(current) = &current {
        layout.install_units(current)?;
    }
    let own = std::env::current_exe()?;
    let first = |program: PathBuf, args: &[&str]| crate::supervise::First {
        program,
        args: args.iter().map(|&arg| arg.to_owned()).collect(),
    };
    let first = if let Some(journal) = Journal::read(&layout.journal())? {
        Some(first(layout.release(&journal.to).join("bin/demi-server"), &["resume"]))
    } else if current.is_none() || crate::setup::marker(layout).exists() {
        Some(first(own, &["set-up-container"]))
    } else if current.as_ref().is_some_and(|current| current < &image) {
        Some(first(own, &["upgrade", &image.to_string(), "--from", ASSETS]))
    } else {
        None
    };
    match first {
        Some(first) => crate::supervise::run(layout, Some(first)),
        None => Err(std::process::Command::new(layout.current().join("bin/demi-server"))
            .arg("--root")
            .arg(layout.root())
            .arg("supervise")
            .exec()
            .into()),
    }
}

/// Writes `config/demi.env` from the container's `DEMI_*` and `AWS_*`
/// variables, in the order of their names.
fn write_configuration(layout: &Layout) -> Result<(), Box<dyn std::error::Error>> {
    let mut variables: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| name.starts_with("DEMI_") || name.starts_with("AWS_"))
        .collect();
    variables.sort();
    let config = layout.config();
    std::fs::create_dir_all(config.parent().expect("the configuration lies in a directory"))?;
    demi_shared_artifacts::publish_bytes_blocking(
        &config,
        Settings::new(variables).file().as_bytes(),
        Publication {
            mode: Publish::Replace,
            permissions: Permissions::Private,
            durable: true,
        },
    )?;
    Ok(())
}
