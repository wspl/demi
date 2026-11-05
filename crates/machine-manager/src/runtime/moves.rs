//! A stopped device's move between runtimes, which keeps everything: its
//! home, and its system with what its user installed.
//!
//! ```text
//! gVisor -> Simple   system image's upper ─┐ read-only    copied into
//!                    pinned base ──────────┴ overlay  ──> system/
//!                    home image ────────────────────────> home/
//!
//! Simple -> gVisor   system/ compared with its base ───> upper/ in a new
//!                    (changes, new entries, whiteouts)    system image
//!                    home/ ──────────────────────────────> a new home image
//! ```
//!
//! The new storage is made whole and recorded before the old one is
//! removed, so a move cut short leaves the device's storage on one runtime
//! or both, and never on neither. A device with both is settled by
//! [`settle`]: the move ended, and the newer record is its result.
//!
//! A Simple Isolation tree records no owners: its files are all its user's.
//! A move to gVisor gives a changed system file its owner in the base and a
//! new one its directory's owner, and the home to the sandbox's user.
//!
//! A move copies every file of the device, which takes minutes for a large
//! home, so it ends early when the manager stops rather than hold up the
//! stop: what it made is in a stage the next start removes, and the next
//! manager moves the device again.

use std::{
    future::Future,
    num::NonZeroU64,
    path::{Path, PathBuf},
    rc::Rc,
    time::SystemTime,
};

use demi_machine_manager_protocol::{
    DeviceId, MachineImageState, Volume,
    image::PROGRAMS_PATH,
};

use tokio_util::sync::CancellationToken;

use super::simple::Contents;
use crate::{
    blocking::{self, OffLoop},
    linux::{loopdev, mount},
    manager::{Core, OpError, admission::Closed},
    sandbox::{files::USER_ID, oci::BOOT_RECORD},
    storage::{
        durable::{remove_tree, sync},
        ext4, room,
        store::{ImagePair, ImageStore, Retention},
        tree,
        working::{WorkingPair, new_generation},
    },
};

/// What a gVisor boot makes anew, or mounts from elsewhere, and a Simple
/// Isolation boot writes into the system: a move to gVisor leaves them out.
const BOOT_FILES: [&str; 5] = [PROGRAMS_PATH, "/run", "/tmp", "/etc/resolv.conf", "/etc/hosts"];

/// The room a new filesystem gets beyond the bytes its files take, for its
/// own structures: the next wake enlarges it to the user's capacity.
fn filesystem_bytes(used: u64) -> NonZeroU64 {
    const MIB: u64 = 1 << 20;
    let bytes = used.saturating_add(used / 4).saturating_add(64 * MIB);
    NonZeroU64::new(bytes.div_ceil(MIB) * MIB).expect("a filesystem has room for its structures")
}

/// A directory the move works in, in the working directory, whose name
/// starts with a dot so the next start removes it.
struct Stage(PathBuf);

impl Stage {
    async fn create(core: &Core) -> Result<Self, OpError> {
        let path = core.config.working().join(format!(".move-{}", uuid::Uuid::new_v4()));
        let created = path.clone();
        blocking::run(move |_| fs_err::create_dir_all(created)).await?;
        Ok(Self(path))
    }

    /// Unmounts what the move mounted in it, the overlay first, and removes
    /// it; what stays behind the next start removes.
    async fn remove(self, mounted: Vec<PathBuf>) {
        let path = self.0;
        let removed = blocking::run(move |off| -> std::io::Result<()> {
            for target in mounted.iter().rev() {
                mount::unmount(off, target)?;
            }
            remove_tree(off, &path)
        })
        .await;
        if let Err(error) = removed {
            tracing::warn!(event = "stage.remove_failed", kind = "move", %error, "a stage that stays behind could not be removed");
        }
    }
}

/// Runs `step` until it ends or `stop` is cancelled; a step cut short drops
/// the tool it runs, which ends its process.
async fn until_stopped<T, E: Into<OpError>>(stop: &CancellationToken, step: impl Future<Output = Result<T, E>>) -> Result<T, OpError> {
    tokio::select! {
        result = step => result.map_err(Into::into),
        () = stop.cancelled() => Err(OpError::Stopping(Closed)),
    }
}

/// Moves `device`'s gVisor storage to Simple Isolation, until `stop` is
/// cancelled.
pub async fn to_simple(core: &Rc<Core>, device: &DeviceId, stop: &CancellationToken) -> Result<(), OpError> {
    let gvisor = core.gvisor.as_ref().map_err(|reason| OpError::GvisorUnavailable(reason.clone()))?;
    let (store, id) = (gvisor.store.clone(), device.clone());
    let Some(state) = blocking::run(move |off| store.read(off, &id)).await? else {
        return Ok(());
    };
    let images = gvisor.store.images(device, &state.generation);
    let base = core.bases().join(&state.base_version).join("rootfs");
    let stage = Stage::create(core).await?;
    let mut mounted = Vec::new();
    let (system, home) = (stage.0.join("system-image"), stage.0.join("home-image"));
    let merged = stage.0.join("system");
    let made = mount_generation(core, device, &images, &base, [&system, &home, &merged], &mut mounted).await;
    let moved = match made {
        Ok(()) => core
            .simple
            .build(
                device,
                &state.base_version,
                state.reset_id.clone(),
                Contents::Moved {
                    system: merged,
                    home,
                    stop: stop.clone(),
                },
                &core.bases(),
            )
            .await
            .map(drop),
        Err(error) => Err(error),
    };
    stage.remove(mounted).await;
    moved?;
    remove_gvisor(core, device).await
}

/// Mounts `images` read-only at the first two of `targets`, and the system
/// as a sandbox sees it, its upper directory over `base`, at the third,
/// recording each mount in `mounted`.
async fn mount_generation(
    core: &Core,
    device: &DeviceId,
    images: &ImagePair<PathBuf>,
    base: &Path,
    targets: [&Path; 3],
    mounted: &mut Vec<PathBuf>,
) -> Result<(), OpError> {
    let (images, base) = (images.clone(), base.to_owned());
    let [system, home, merged] = targets.map(Path::to_path_buf);
    let (store, id) = (core.gvisor().store.clone(), device.clone());
    let made = blocking::run(move |off| -> (Vec<PathBuf>, Result<(), OpError>) {
        let mut mounted = Vec::new();
        let result = (|| -> Result<(), OpError> {
            // Both copies exist until the move ends, and the new system
            // holds the base as well as what the image changes in it.
            let needed = tree::used_bytes(off, images.system.parent().expect("an image lies in its generation"))?
                .saturating_add(tree::used_bytes(off, &base)?);
            let _room = room::ensure(off, store.root(), needed, |off| {
                store.drop_fallback(off, &id).map_err(std::io::Error::other)
            })
            .map_err(std::io::Error::other)?;
            for (image, target) in [(&images.system, &system), (&images.home, &home)] {
                fs_err::create_dir(target)?;
                let device = loopdev::attach_read_only(off, image)?;
                mount::ext4_read_only(off, &device.path(), target)?;
                mounted.push(target.clone());
            }
            fs_err::create_dir(&merged)?;
            let upper = system.join("upper");
            let lowers: Vec<&Path> = match upper.is_dir() {
                true => vec![&upper, &base],
                false => vec![&base],
            };
            mount::read_only_overlay(off, &lowers, &merged)?;
            mounted.push(merged.clone());
            Ok(())
        })();
        (mounted, result)
    })
    .await;
    mounted.extend(made.0);
    made.1
}

/// Removes `device`'s gVisor storage: its generations and any working pair.
async fn remove_gvisor(core: &Core, device: &DeviceId) -> Result<(), OpError> {
    let store = ImageStore::new(core.config.images());
    let working = WorkingPair::new(&core.config.working(), device);
    let id = device.clone();
    blocking::run(move |off| -> Result<(), OpError> {
        remove_tree(off, working.directory())?;
        store.remove(off, &id)?;
        Ok(())
    })
    .await
}

/// Moves `device`'s Simple Isolation tree to gVisor, as a generation of its
/// own, until `stop` is cancelled.
pub async fn to_gvisor(core: &Rc<Core>, device: &DeviceId, stop: &CancellationToken) -> Result<(), OpError> {
    let gvisor = core.gvisor.as_ref().map_err(|reason| OpError::GvisorUnavailable(reason.clone()))?;
    let (trees, id) = (core.simple.trees.clone(), device.clone());
    let Some(record) = blocking::run(move |off| trees.read(off, &id)).await? else {
        return Ok(());
    };
    let base = core.bases().join(&record.base_version).join("rootfs");
    if !base.is_dir() {
        return Err(OpError::MissingBase(record.base_version));
    }
    let stage = Stage::create(core).await?;
    let made = make_generation(core, device, &base, &stage.0, stop).await;
    let published = match made {
        Ok(images) => {
            let state = MachineImageState {
                generation: new_generation(),
                base_version: record.base_version.clone(),
                reset_id: record.reset_id.clone(),
                system_bytes: until_stopped(stop, ext4::recover(&core.tools, &images.system)).await?,
                home_bytes: until_stopped(stop, ext4::recover(&core.tools, &images.home)).await?,
            };
            let (store, id) = (gvisor.store.clone(), device.clone());
            blocking::run(move |off| -> Result<(), OpError> {
                // The device's first generation: there is no older one to keep.
                store.publish(off, &id, &state, images.as_deref(), Retention::Current)?;
                Ok(())
            })
            .await
        }
        Err(error) => Err(error),
    };
    stage.remove(Vec::new()).await;
    published?;
    core.simple.remove(device).await
}

/// Makes the system and home images of `device`'s tree in `stage`.
async fn make_generation(
    core: &Core,
    device: &DeviceId,
    base: &Path,
    stage: &Path,
    stop: &CancellationToken,
) -> Result<ImagePair<PathBuf>, OpError> {
    let tree = core.simple.tree(device);
    let (system_root, home_root) = (stage.join("system-root"), stage.join("home-root"));
    let (base, store) = (base.to_owned(), core.gvisor().store.clone());
    let copying = stop.clone();
    let roots = (system_root.clone(), home_root.clone());
    let used = blocking::run(move |off| -> Result<(u64, u64), OpError> {
        let (system_root, home_root) = roots;
        let needed = tree::used_bytes(off, &tree.system())?.saturating_add(tree::used_bytes(off, &tree.home())?);
        // The two copies, and the two images made of the second.
        let _room = room::ensure(off, store.root(), needed.saturating_mul(2), |_| Ok(false)).map_err(std::io::Error::other)?;
        fs_err::create_dir(&system_root)?;
        let skip: Vec<PathBuf> = BOOT_FILES
            .iter()
            .chain(std::iter::once(&BOOT_RECORD))
            .map(|path| PathBuf::from(path.trim_start_matches('/')))
            .collect();
        let skip: Vec<&Path> = skip.iter().map(PathBuf::as_path).collect();
        tree::upper(off, &base, &tree.system(), &system_root.join("upper"), &skip, &copying)?;
        // The home filesystem's root is the sandbox's /home, root's, and
        // what is in it the sandbox's user's.
        fs_err::create_dir(&home_root)?;
        for entry in fs_err::read_dir(tree.home())? {
            let entry = entry?;
            tree::copy(off, &entry.path(), &home_root.join(entry.file_name()), Some(USER_ID), Some(&copying))?;
        }
        use std::os::unix::fs::PermissionsExt as _;
        fs_err::set_permissions(&home_root, std::fs::Permissions::from_mode(0o755))?;
        Ok((tree::used_bytes(off, &system_root)?, tree::used_bytes(off, &home_root)?))
    })
    .await?;
    let images = ImagePair::in_directory(stage);
    until_stopped(stop, ext4::make_populated(&core.tools, Volume::System, &system_root, &images.system, filesystem_bytes(used.0))).await?;
    until_stopped(stop, ext4::make_populated(&core.tools, Volume::Home, &home_root, &images.home, filesystem_bytes(used.1))).await?;
    let synced = images.clone();
    blocking::run(move |off| -> std::io::Result<()> {
        for volume in Volume::ALL {
            sync(off, synced.get(volume))?;
        }
        Ok(())
    })
    .await?;
    Ok(images)
}

/// Settles a device whose storage is on both runtimes, which only a move
/// cut short before it removed the old storage leaves: the storage recorded
/// last is the move's result, and the other goes.
pub async fn settle(core: &Rc<Core>, device: &DeviceId) -> Result<(), OpError> {
    let simple = core.simple.tree(device);
    let store = core.config.images().join(device.as_str()).join("current.json");
    let record = simple.record();
    let simple_newer = blocking::run(move |_: &OffLoop| -> std::io::Result<bool> {
        let modified = |path: &Path| fs_err::metadata(path).and_then(|metadata| metadata.modified());
        Ok(modified(&record)? > modified(&store).unwrap_or(SystemTime::UNIX_EPOCH))
    })
    .await?;
    tracing::warn!(event = "cloud_move.settled", host_id = %device, kept = if simple_newer { "simple" } else { "gvisor" }, "a Cloud's storage was on both runtimes after a move was cut short; the newer was kept");
    match simple_newer {
        true => remove_gvisor(core, device).await,
        false => core.simple.remove(device).await,
    }
}
