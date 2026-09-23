//! A new home's first contents (`managed-hosts.md` § Container
//! initialization): the base's `/etc/skel`, copied with symbolic links as
//! they are and every entry owned by the sandbox's user.

use std::{io, path::Path};

use crate::{blocking::OffLoop, sandbox::files::USER_ID};

/// Copies the directory `skeleton` to the new directory `destination`.
pub fn copy(_: &OffLoop, skeleton: &Path, destination: &Path) -> io::Result<()> {
    for entry in walkdir::WalkDir::new(skeleton).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        let relative = entry
            .path()
            .strip_prefix(skeleton)
            .expect("a walked entry lies under its root");
        let target = destination.join(relative);
        let kind = entry.file_type();
        if kind.is_dir() {
            fs_err::create_dir(&target)?;
            fs_err::set_permissions(&target, entry.metadata()?.permissions())?;
        } else if kind.is_symlink() {
            fs_err::os::unix::fs::symlink(fs_err::read_link(entry.path())?, &target)?;
        } else if kind.is_file() {
            fs_err::copy(entry.path(), &target)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is neither a file, a directory nor a link", entry.path().display()),
            ));
        }
        fs_err::os::unix::fs::lchown(&target, Some(USER_ID), Some(USER_ID))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn links_are_copied_as_they_are_and_everything_belongs_to_the_user() {
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let skeleton = directory.path().join("skel");
        std::fs::create_dir_all(skeleton.join(".config")).unwrap();
        std::fs::write(skeleton.join(".profile"), "PATH=$HOME/.local/bin:$PATH\n").unwrap();
        std::os::unix::fs::symlink(".profile", skeleton.join(".bash_profile")).unwrap();
        std::os::unix::fs::symlink("/etc/bash.bashrc", skeleton.join(".bashrc")).unwrap();
        let home = directory.path().join("demi");
        let copied = copy(&off, &skeleton, &home);
        // Giving the entries to the user needs root.
        if !rustix::process::geteuid().is_root() {
            assert_eq!(copied.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            return;
        }
        copied.unwrap();
        assert_eq!(std::fs::read_link(home.join(".bash_profile")).unwrap(), Path::new(".profile"));
        assert_eq!(std::fs::read_link(home.join(".bashrc")).unwrap(), Path::new("/etc/bash.bashrc"));
        for path in [home.clone(), home.join(".config"), home.join(".profile"), home.join(".bashrc")] {
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert_eq!((metadata.uid(), metadata.gid()), (USER_ID, USER_ID), "{}", path.display());
        }
    }

    /// A new home image's root is `/home` in the sandbox: mode 0755 whatever
    /// the service's umask, with the user's directory inside.
    #[tokio::test]
    #[ignore = "needs root: run the Linux suite with --ignored as root"]
    async fn a_new_home_image_holds_the_users_directory_under_a_readable_root() {
        use std::os::unix::fs::PermissionsExt;
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let skeleton = directory.path().join("skel");
        std::fs::create_dir(&skeleton).unwrap();
        std::fs::write(skeleton.join(".profile"), "export EDITOR=vi\n").unwrap();
        let root = directory.path().join("mkhome");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        copy(&off, &skeleton, &root.join("demi")).unwrap();
        let image = directory.path().join("home.ext4");
        let tools = crate::tools::Tools::on_path();
        crate::storage::ext4::make_home(&tools, &root, &image, std::num::NonZeroU64::new(32 << 20).unwrap())
            .await
            .unwrap();
        let stat = |path: &str| {
            let output = std::process::Command::new("debugfs")
                .args(["-R", &format!("stat {path}")])
                .arg(&image)
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        let top = stat("/");
        assert!(top.contains("Mode:  0755"), "{top}");
        let home = stat("/demi");
        assert!(home.contains("User:  1000") && home.contains("Group:  1000"), "{home}");
        let profile = stat("/demi/.profile");
        assert!(profile.contains("User:  1000"), "{profile}");
    }
}
