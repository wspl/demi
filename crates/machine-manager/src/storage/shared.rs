//! The integrations' shared folders (`demi_machine_manager_protocol::shared`).
//!
//! ```text
//! <data>/shared/<id>/shared.ext4                      the folder's image
//! /run/demi-machine-manager/shared/<id>               its one mount, while any
//!                                                     sandbox that names it runs
//! <data>/shared-files/<id>/                           the folder on Simple
//!                                                     Isolation, a directory
//! ```
//!
//! A folder is in one of those two forms, the one the isolation of the Clouds
//! that use it takes. When the master switches the isolation, the first Cloud
//! that wakes on the other one, once no Cloud on the first one uses the
//! folder, moves it to its form, and a wake while one does is refused.
//!
//! A move makes the new form whole under another name, renames the old form
//! to `.<id>.retired` beside it, renames the new one into place, and removes
//! the retired one, so the folder never has both forms at once: each would
//! be taken for the folder by the Clouds of its isolation. A crash midway
//! leaves a retired form, which the next first use puts back when the new
//! form is not in place, and removes when it is.
//!
//! Under a manager that runs as root each Simple Isolation Cloud is a user of
//! its own, so the directory and what is in it belong to [`GROUP`], which the
//! Clouds that name a folder run with as their own group, with a umask that
//! keeps what they make the group's to write.
//!
//! An ext4 filesystem is mounted once: two mounts of one image would each
//! cache it and corrupt it. So the manager mounts a folder's image on the
//! server when the first sandbox that names it starts, binds that mount into
//! each such sandbox, and unmounts it when the last one stops. gVisor's
//! shared file access (`--file-access-mounts=shared`) has each sandbox see
//! the others' writes. A sandbox's start and stop hold the folder's lock,
//! so two workers never mount it twice.
//!
//! The image is no generation: a stop does not save it and a reset keeps it,
//! as a home's working image would be if nothing published it. Its journal
//! keeps it whole across a crash, and the check before each mount repairs
//! what the journal does not.
//!
//! A manager that starts unmounts each folder an earlier one left
//! mounted, after it fenced every sandbox (`crate::recovery`), but the
//! folders of the sandboxes it took over (`crate::handover`), which it
//! counts for them.

use std::{
    cell::RefCell,
    collections::HashMap,
    fs::Permissions,
    io,
    num::NonZeroU64,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    rc::Rc,
};

use demi_machine_manager_protocol::{Isolation, is_image_name};

use crate::{
    blocking::{self, OffLoop},
    linux::{loopdev, mount},
    manager::Core,
    runtime::{moves::filesystem_bytes, simple::LOST_AND_FOUND},
    sandbox::files::USER_ID,
    storage::{
        durable::{create_private, remove_tree, sync},
        ext4::{self, Ext4Error},
        room, tree,
    },
    tools::ToolError,
};

/// The group of the Simple Isolation folders under a manager that runs as
/// root: Debian reserves 60000 to 64999 for no package or person, and the
/// devices' users start at 61000.
pub const GROUP: u32 = 60999;

#[derive(Debug, thiserror::Error)]
pub enum SharedError {
    #[error("{0} is no shared folder's name")]
    Name(String),
    #[error("the shared folder {0} is in use by Clouds on the other isolation until they stop")]
    OtherIsolation(String),
    #[error("the shared folder {0} is mounted by a running Cloud")]
    Mounted(String),
    #[error("the shared folder {0} needs recovery: {1}")]
    NeedsRecovery(String, String),
    #[error(transparent)]
    Ext4(#[from] Ext4Error),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// One folder in use: how many Clouds use it, and on which isolation.
#[derive(Default)]
struct Held {
    users: usize,
    on: Option<Isolation>,
}

/// The folders in use, each behind its lock.
#[derive(Default)]
pub struct SharedFolders {
    folders: RefCell<HashMap<String, Rc<tokio::sync::Mutex<Held>>>>,
}

impl SharedFolders {
    fn folder(&self, id: &str) -> Rc<tokio::sync::Mutex<Held>> {
        self.folders.borrow_mut().entry(id.to_owned()).or_default().clone()
    }

    /// Counts the folder `id` for a Cloud on `isolation` taken over that
    /// uses it, which an earlier manager prepared.
    pub async fn adopt(&self, id: &str, isolation: Isolation) {
        let folder = self.folder(id);
        let mut held = folder.lock().await;
        held.users += 1;
        held.on = Some(isolation);
    }

    /// Where a Cloud on `isolation` finds the folder `id` on the server, for
    /// one more Cloud: when none has it yet, its form is made ready, a
    /// gVisor image at `capacity` or larger, and a folder in the other form
    /// moves to it. A Simple Isolation folder has no size of its own.
    pub async fn acquire(
        &self,
        core: &Core,
        isolation: Isolation,
        id: &str,
        capacity: NonZeroU64,
    ) -> Result<PathBuf, SharedError> {
        if !is_image_name(id) {
            return Err(SharedError::Name(id.to_owned()));
        }
        let folder = self.folder(id);
        let mut held = folder.lock().await;
        if held.users > 0 && held.on != Some(isolation) {
            return Err(SharedError::OtherIsolation(id.to_owned()));
        }
        if held.users == 0 {
            finish_move(core, id).await?;
        }
        let path = match (isolation, held.users) {
            (Isolation::Gvisor, 0) => mount_image(core, id, capacity).await?,
            (Isolation::Gvisor, _) => mount_point(core, id),
            (Isolation::Simple, 0) => open_directory(core, id).await?,
            (Isolation::Simple, _) => files(core, id),
        };
        held.users += 1;
        held.on = Some(isolation);
        Ok(path)
    }

    /// One Cloud of the folder `id` stopped: the last one's stop unmounts
    /// an image, which syncs it. The Cloud has let go whether or not the
    /// unmount succeeds: a mount left behind is used again by the next
    /// start, and unmounted by a deletion or the manager's next start.
    pub async fn release(&self, core: &Core, id: &str) -> Result<(), SharedError> {
        let folder = self.folder(id);
        let mut held = folder.lock().await;
        if held.users == 0 {
            return Ok(());
        }
        held.users -= 1;
        if held.users == 0 && held.on.take() == Some(Isolation::Gvisor) {
            let target = mount_point(core, id);
            blocking::run(move |off| unmount_if_mounted(off, &target)).await?;
        }
        Ok(())
    }

    /// Removes the folder `id`, in either form; refused while a Cloud has it.
    pub async fn delete(&self, core: &Core, id: &str) -> Result<(), SharedError> {
        if !is_image_name(id) {
            return Err(SharedError::Name(id.to_owned()));
        }
        let folder = self.folder(id);
        let held = folder.lock().await;
        if held.users > 0 {
            return Err(SharedError::Mounted(id.to_owned()));
        }
        let directory = directory_of(&image(core, id));
        let root = shared_root(core);
        let target = mount_point(core, id);
        // Each form, the stage of a directory and what a move retired.
        let forms = [
            files(core, id),
            core.config.shared_files().join(format!(".{id}")),
            retired_image(core, id),
            retired_files(core, id),
        ];
        blocking::run(move |off| -> io::Result<()> {
            // A mount an earlier stop failed to unmount goes first, then
            // its mount point.
            unmount_if_mounted(off, &target)?;
            match fs_err::remove_dir(&target) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            remove_tree(off, &directory)?;
            for form in &forms {
                remove_tree(off, form)?;
            }
            match sync(off, &root) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            }
        })
        .await?;
        Ok(())
    }
}

/// Ends the move of the folder `id` that a crash cut short: a retired form
/// comes back when the new one is not in place, and goes when it is. A
/// folder found in both forms needs recovery, since either may hold what the
/// other lacks.
async fn finish_move(core: &Core, id: &str) -> Result<(), SharedError> {
    let (image, files) = (image(core, id), files(core, id));
    let (retired_image, retired_files) = (retired_image(core, id), retired_files(core, id));
    let both = blocking::run(move |off| -> io::Result<bool> {
        if retired_image.exists() {
            if files.is_dir() {
                remove_tree(off, &retired_image)?;
            } else {
                fs_err::rename(&retired_image, directory_of(&image))?;
                sync(off, directory_of(&retired_image).as_path())?;
            }
        }
        if retired_files.exists() {
            if image.exists() {
                remove_tree(off, &retired_files)?;
            } else {
                fs_err::rename(&retired_files, &files)?;
                sync(off, directory_of(&retired_files).as_path())?;
            }
        }
        Ok(image.exists() && files.is_dir())
    })
    .await?;
    match both {
        true => Err(SharedError::NeedsRecovery(id.to_owned(), "it has both an image and a directory".to_owned())),
        false => Ok(()),
    }
}

/// Mounts the folder `id`'s image, which no Cloud has: made at `capacity`,
/// of its Simple Isolation directory if it has one, on its first use,
/// enlarged when smaller, and checked.
async fn mount_image(core: &Core, id: &str, capacity: NonZeroU64) -> Result<PathBuf, SharedError> {
    let target = mount_point(core, id);
    let image = image(core, id);
    let directory = image.parent().expect("an image lies in its directory").to_owned();
    let (made, directory_form) = (image.clone(), files(core, id));
    let held_form = directory_form.clone();
    let (exists, of_directory) = blocking::run(move |off| -> io::Result<(bool, bool)> {
        create_private(off, &directory)?;
        Ok((made.exists(), held_form.is_dir()))
    })
    .await?;
    if !exists {
        // Made under another name and renamed, so a crash midway
        // leaves no image a later start would take for whole; the
        // partial one is made again over.
        let partial = image.with_extension("ext4.partial");
        if of_directory {
            make_of_directory(core, &directory_form, &partial, capacity).await?;
        } else {
            ext4::make_shared(&core.tools, &partial, capacity, USER_ID).await?;
        }
        let (made, directory) = (image.clone(), directory_of(&image));
        let retired = retired_files(core, id);
        blocking::run(move |off| -> io::Result<()> {
            sync(off, &partial)?;
            if of_directory {
                fs_err::rename(&directory_form, &retired)?;
                sync(off, &directory_of(&retired))?;
            }
            fs_err::rename(&partial, &made)?;
            sync(off, &directory)?;
            remove_tree(off, &retired)
        })
        .await?;
    }
    let output = core
        .tools
        .output(crate::tools::Tool::E2fsck, [Path::new("-p"), &image], None)
        .await?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(SharedError::NeedsRecovery(id.to_owned(), output.message().to_owned()));
    }
    let held = blocking::run({
        let image = image.clone();
        move |off| ext4::capacity(off, &image)
    })
    .await?;
    if held < capacity {
        ext4::enlarge(&core.tools, &image, capacity).await?;
    }
    let mounted = target.clone();
    blocking::run(move |off| -> io::Result<()> {
        create_private(off, mounted.parent().expect("a mount point lies in its directory"))?;
        match fs_err::create_dir(&mounted) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
        // A mount whose last unmount failed is still there, whole:
        // it serves again.
        if mount::mount_root(off, &mounted)? == Some(true) {
            return Ok(());
        }
        let device = loopdev::attach(off, &image)?;
        mount::ext4(off, &device.path(), &mounted)
    })
    .await?;
    Ok(target)
}

/// Makes the image `partial`, at `capacity` or as large as it takes, of the
/// directory `from`, with every file the sandboxes' user's.
async fn make_of_directory(core: &Core, from: &Path, partial: &Path, capacity: NonZeroU64) -> Result<(), SharedError> {
    let stage = partial.with_extension("stage");
    let (source, copy) = (from.to_owned(), stage.clone());
    let used = blocking::run(move |off| -> io::Result<u64> {
        remove_tree(off, &copy)?;
        let used = tree::used_bytes(off, &source)?;
        // The copy, and the image made of it.
        let _room = room::ensure(off, &source, used.saturating_mul(2), |_| Ok(false)).map_err(io::Error::other)?;
        tree::copy(off, &source, &copy, Some(USER_ID), None)?;
        Ok(used)
    })
    .await?;
    let bytes = capacity.max(filesystem_bytes(used));
    let made = ext4::make_shared_of(&core.tools, &stage, partial, bytes, USER_ID).await;
    let removed = blocking::run(move |off| remove_tree(off, &stage)).await;
    made?;
    Ok(removed?)
}

/// Readies the folder `id`'s directory, which no Cloud has: new, or of its
/// gVisor image if it has one, which then goes. It is made under another
/// name and renamed, so a crash midway leaves no directory a later start
/// would take for whole.
async fn open_directory(core: &Core, id: &str) -> Result<PathBuf, SharedError> {
    let (root, directory) = (core.config.shared_files(), files(core, id));
    let stage = root.join(format!(".{id}"));
    let users = core.simple.runs_as_users();
    let image = image(core, id);
    let (exists, has_image) = blocking::run({
        let (stage, directory, image) = (stage.clone(), directory.clone(), image.clone());
        move |off| -> io::Result<(bool, bool)> {
            match fs_err::create_dir(&root) {
                Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
                _ => {}
            }
            let mode = if users { 0o711 } else { 0o700 };
            fs_err::set_permissions(&root, Permissions::from_mode(mode))?;
            remove_tree(off, &stage)?;
            Ok((directory.is_dir(), image.exists()))
        }
    })
    .await?;
    if exists {
        return Ok(directory);
    }
    if has_image {
        let output = core
            .tools
            .output(crate::tools::Tool::E2fsck, [Path::new("-p"), &image], None)
            .await?;
        if !matches!(output.status.code(), Some(0 | 1)) {
            return Err(SharedError::NeedsRecovery(id.to_owned(), output.message().to_owned()));
        }
    }
    let target = mount_point(core, id);
    let made = directory.clone();
    let retired = retired_image(core, id);
    blocking::run(move |off| -> io::Result<()> {
        if has_image {
            copy_image(off, &image, &target, &stage)?;
        } else {
            fs_err::create_dir(&stage)?;
        }
        if users {
            open_to_group(&stage)?;
        }
        // Only the Clouds that name the folder enter it, whatever mode the
        // image's root had.
        let mode = if users { 0o770 } else { 0o700 };
        fs_err::set_permissions(&stage, Permissions::from_mode(mode))?;
        sync(off, &stage)?;
        if has_image {
            fs_err::rename(directory_of(&image), &retired)?;
            sync(off, &directory_of(&retired))?;
        }
        fs_err::rename(&stage, &made)?;
        sync(off, made.parent().expect("a folder lies in its directory"))?;
        remove_tree(off, &retired)
    })
    .await?;
    Ok(directory)
}

/// Copies the files of the image `image` into the new directory `to`,
/// mounting it read-only at `target` for the copy.
fn copy_image(off: &OffLoop, image: &Path, target: &Path, to: &Path) -> io::Result<()> {
    create_private(off, target.parent().expect("a mount point lies in its directory"))?;
    match fs_err::create_dir(target) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
        _ => {}
    }
    unmount_if_mounted(off, target)?;
    let device = loopdev::attach_read_only(off, image)?;
    mount::ext4_read_only(off, &device.path(), target)?;
    let copied = (|| -> io::Result<()> {
        let used = tree::used_bytes(off, target)?;
        let _room = room::ensure(off, directory_of(to).as_path(), used, |_| Ok(false)).map_err(io::Error::other)?;
        tree::copy(off, target, to, Some(0), None)?;
        remove_tree(off, &to.join(LOST_AND_FOUND))
    })();
    let unmounted = unmount_if_mounted(off, target);
    copied.and(unmounted)
}

/// Gives everything in the directory `root`, itself included, to [`GROUP`],
/// with what its owner may do.
fn open_to_group(root: &Path) -> io::Result<()> {
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        let group = Some(rustix::process::Gid::from_raw(GROUP));
        rustix::fs::chownat(
            rustix::fs::CWD,
            entry.path(),
            None,
            group,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )?;
        if entry.path_is_symlink() {
            continue;
        }
        let mode = entry.metadata()?.permissions().mode();
        let opened = mode | ((mode & 0o700) >> 3);
        let opened = if entry.file_type().is_dir() { opened | 0o070 } else { opened };
        fs_err::set_permissions(entry.path(), Permissions::from_mode(opened))?;
    }
    Ok(())
}

/// Unmounts every folder an earlier manager left mounted but those in
/// `kept`, which sandboxes taken over use; only a start, after every other
/// sandbox was fenced, calls it.
pub async fn unmount_left(core: &Core, kept: &[String]) -> io::Result<()> {
    let root = core.config.runtime().join("shared");
    let kept = kept.to_vec();
    blocking::run(move |off| -> io::Result<()> {
        let entries = match fs_err::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if !kept.iter().any(|id| entry.file_name() == id.as_str()) {
                unmount_if_mounted(off, &entry.path())?;
            }
        }
        Ok(())
    })
    .await
}

fn unmount_if_mounted(off: &OffLoop, target: &Path) -> io::Result<()> {
    if mount::mount_root(off, target)? == Some(true) {
        mount::unmount(off, target)?;
    }
    Ok(())
}

fn directory_of(image: &Path) -> PathBuf {
    image.parent().expect("an image lies in its directory").to_owned()
}

fn files(core: &Core, id: &str) -> PathBuf {
    core.config.shared_files().join(id)
}

fn shared_root(core: &Core) -> PathBuf {
    core.config.shared()
}

fn image(core: &Core, id: &str) -> PathBuf {
    shared_root(core).join(id).join("shared.ext4")
}

/// The image's directory while a move to a directory retires it.
fn retired_image(core: &Core, id: &str) -> PathBuf {
    shared_root(core).join(format!(".{id}.retired"))
}

/// The directory while a move to an image retires it.
fn retired_files(core: &Core, id: &str) -> PathBuf {
    core.config.shared_files().join(format!(".{id}.retired"))
}

fn mount_point(core: &Core, id: &str) -> PathBuf {
    core.config.runtime().join("shared").join(id)
}
