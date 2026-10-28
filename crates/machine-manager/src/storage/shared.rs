//! The integrations' shared folders (`demi_machine_manager_protocol::shared`).
//!
//! ```text
//! <data>/shared/<id>/shared.ext4                      the folder's image
//! /run/demi-machine-manager/shared/<id>               its one mount, while any
//!                                                     sandbox that names it runs
//! ```
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
//! mounted, after it fenced every sandbox (`crate::recovery`).

use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    num::NonZeroU64,
    path::{Path, PathBuf},
    rc::Rc,
};

use demi_machine_manager_protocol::is_image_name;

use crate::{
    blocking::{self, OffLoop},
    linux::{loopdev, mount},
    manager::Core,
    sandbox::files::USER_ID,
    storage::{
        durable::{create_private, remove_tree, sync},
        ext4::{self, Ext4Error},
    },
    tools::ToolError,
};

#[derive(Debug, thiserror::Error)]
pub enum SharedError {
    #[error("{0} is no shared folder's name")]
    Name(String),
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

/// The mounted folders: how many sandboxes each serves, behind its lock.
#[derive(Default)]
pub struct SharedImages {
    folders: RefCell<HashMap<String, Rc<tokio::sync::Mutex<usize>>>>,
}

impl SharedImages {
    fn folder(&self, id: &str) -> Rc<tokio::sync::Mutex<usize>> {
        self.folders.borrow_mut().entry(id.to_owned()).or_default().clone()
    }

    /// The server's mount of the folder `id` for one more sandbox: the
    /// image is made at `capacity` on its first use, enlarged when smaller,
    /// checked and mounted when no sandbox has it yet.
    pub async fn acquire(&self, core: &Core, id: &str, capacity: NonZeroU64) -> Result<PathBuf, SharedError> {
        if !is_image_name(id) {
            return Err(SharedError::Name(id.to_owned()));
        }
        let folder = self.folder(id);
        let mut users = folder.lock().await;
        let target = mount_point(core, id);
        if *users == 0 {
            let image = image(core, id);
            let directory = image.parent().expect("an image lies in its directory").to_owned();
            let made = image.clone();
            let exists = blocking::run(move |off| -> io::Result<bool> {
                create_private(off, &directory)?;
                Ok(made.exists())
            })
            .await?;
            if !exists {
                // Made under another name and renamed, so a crash midway
                // leaves no image a later start would take for whole; the
                // partial one is made again over.
                let partial = image.with_extension("ext4.partial");
                ext4::make_shared(&core.tools, &partial, capacity, USER_ID).await?;
                let (made, directory) = (image.clone(), directory_of(&image));
                blocking::run(move |off| -> io::Result<()> {
                    sync(off, &partial)?;
                    fs_err::rename(&partial, &made)?;
                    sync(off, &directory)
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
        }
        *users += 1;
        Ok(target)
    }

    /// One sandbox of the folder `id` stopped: the last one's stop
    /// unmounts it, which syncs it. The sandbox has let go whether or not
    /// the unmount succeeds: a mount left behind is used again by the next
    /// start, and unmounted by a deletion or the manager's next start.
    pub async fn release(&self, core: &Core, id: &str) -> Result<(), SharedError> {
        let folder = self.folder(id);
        let mut users = folder.lock().await;
        if *users == 0 {
            return Ok(());
        }
        *users -= 1;
        if *users == 0 {
            let target = mount_point(core, id);
            blocking::run(move |off| unmount_if_mounted(off, &target)).await?;
        }
        Ok(())
    }

    /// Removes the folder `id`'s image; refused while a sandbox has it.
    pub async fn delete(&self, core: &Core, id: &str) -> Result<(), SharedError> {
        if !is_image_name(id) {
            return Err(SharedError::Name(id.to_owned()));
        }
        let folder = self.folder(id);
        let users = folder.lock().await;
        if *users > 0 {
            return Err(SharedError::Mounted(id.to_owned()));
        }
        let directory = directory_of(&image(core, id));
        let root = shared_root(core);
        let target = mount_point(core, id);
        blocking::run(move |off| -> io::Result<()> {
            // A mount an earlier stop failed to unmount goes first, then
            // its mount point.
            unmount_if_mounted(off, &target)?;
            match fs_err::remove_dir(&target) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            remove_tree(off, &directory)?;
            match sync(off, &root) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            }
        })
        .await?;
        Ok(())
    }
}

/// Unmounts every folder an earlier manager left mounted; only a start,
/// after every sandbox was fenced, calls it.
pub async fn unmount_left(core: &Core) -> io::Result<()> {
    let root = core.config.runtime().join("shared");
    blocking::run(move |off| -> io::Result<()> {
        let entries = match fs_err::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            unmount_if_mounted(off, &entry?.path())?;
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

fn shared_root(core: &Core) -> PathBuf {
    core.config.shared()
}

fn image(core: &Core, id: &str) -> PathBuf {
    shared_root(core).join(id).join("shared.ext4")
}

fn mount_point(core: &Core, id: &str) -> PathBuf {
    core.config.runtime().join("shared").join(id)
}
