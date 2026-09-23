//! A device's ext4 images (`managed-hosts.md` § Images): making the two
//! filesystems, reading a capacity from the superblock, and recovering an
//! unmounted image before it is published.

use std::{
    ffi::OsStr, fs::File, io, num::NonZeroU64, os::unix::fs::FileExt, path::Path, time::Duration,
};

use crate::{
    blocking::{self, OffLoop},
    storage::durable::sync,
    tools::{Tool, ToolError, Tools},
};

/// `mke2fs` makes an empty filesystem in well under a minute.
const MKE2FS_DEADLINE: Duration = Duration::from_secs(60);

/// The superblock's place and the fields the capacity needs.
const SUPERBLOCK_OFFSET: u64 = 1024;
const SUPERBLOCK_BYTES: usize = 1024;
const MAGIC: u16 = 0xEF53;
const INCOMPAT_64BIT: u32 = 0x80;

#[derive(Debug, thiserror::Error)]
pub enum Ext4Error {
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{0} is not an ext4 image")]
    NotExt4(String),
}

/// A filesystem's capacity: its block count times its block size, read from
/// the primary superblock (the same numbers `dumpe2fs -h` prints).
pub fn capacity(_: &OffLoop, image: &Path) -> Result<NonZeroU64, Ext4Error> {
    let file = File::open(image)?;
    let mut superblock = [0; SUPERBLOCK_BYTES];
    file.read_exact_at(&mut superblock, SUPERBLOCK_OFFSET)?;
    parse_capacity(&superblock).ok_or_else(|| Ext4Error::NotExt4(image.display().to_string()))
}

fn parse_capacity(superblock: &[u8; SUPERBLOCK_BYTES]) -> Option<NonZeroU64> {
    let u32_at = |offset: usize| {
        u32::from_le_bytes(superblock[offset..offset + 4].try_into().expect("four bytes"))
    };
    let magic = u16::from_le_bytes([superblock[0x38], superblock[0x39]]);
    let log_block_size = u32_at(0x18);
    if magic != MAGIC || log_block_size > 6 {
        return None;
    }
    let mut blocks = u64::from(u32_at(0x04));
    if u32_at(0x60) & INCOMPAT_64BIT != 0 {
        blocks |= u64::from(u32_at(0x150)) << 32;
    }
    let block_size = 1024_u64 << log_block_size;
    NonZeroU64::new(blocks.checked_mul(block_size)?)
}

/// `mke2fs`'s size argument: whole KiB, rounded up.
fn kibibytes(bytes: NonZeroU64) -> String {
    format!("{}k", bytes.get().div_ceil(1024))
}

/// An empty system filesystem; the overlay's upper and work directories are
/// made when it is first mounted.
pub async fn make_system(tools: &Tools, image: &Path, bytes: NonZeroU64) -> Result<(), Ext4Error> {
    let size = kibibytes(bytes);
    let args = [
        OsStr::new("-q"),
        OsStr::new("-t"),
        OsStr::new("ext4"),
        OsStr::new("-F"),
        OsStr::new("-L"),
        OsStr::new("system"),
        image.as_os_str(),
        OsStr::new(&size),
    ];
    tools.run(Tool::Mke2fs, args, Some(MKE2FS_DEADLINE)).await?;
    Ok(())
}

/// A home filesystem whose root is populated from `root`, ownership and
/// modes included.
pub async fn make_home(tools: &Tools, root: &Path, image: &Path, bytes: NonZeroU64) -> Result<(), Ext4Error> {
    let size = kibibytes(bytes);
    let args = [
        OsStr::new("-q"),
        OsStr::new("-t"),
        OsStr::new("ext4"),
        OsStr::new("-F"),
        OsStr::new("-L"),
        OsStr::new("home"),
        OsStr::new("-d"),
        root.as_os_str(),
        image.as_os_str(),
        OsStr::new(&size),
    ];
    tools.run(Tool::Mke2fs, args, Some(MKE2FS_DEADLINE)).await?;
    Ok(())
}

/// Checks an unmounted image and completes a growth that was interrupted
/// after its file was extended, then syncs the image and returns its
/// capacity. A check or a resize runs as long as it needs.
pub async fn recover(tools: &Tools, image: &Path) -> Result<NonZeroU64, Ext4Error> {
    check(tools, image, "-p").await?;
    let path = image.to_owned();
    let (length, filesystem) = blocking::run(move |off| -> Result<_, Ext4Error> {
        Ok((fs_err::metadata(&path)?.len(), capacity(off, &path)?))
    })
    .await?;
    if length > filesystem.get() {
        check(tools, image, "-pf").await?;
        tools.run(Tool::Resize2fs, [image], None).await?;
    }
    let path = image.to_owned();
    blocking::run(move |off| {
        sync(off, &path)?;
        capacity(off, &path)
    })
    .await
}

/// `e2fsck` with `mode`; exit 1 means it corrected the filesystem.
async fn check(tools: &Tools, image: &Path, mode: &str) -> Result<(), Ext4Error> {
    let args = [OsStr::new(mode), image.as_os_str()];
    let output = tools.output(Tool::E2fsck, args, None).await?;
    Tools::accept(Tool::E2fsck, output, &[0, 1])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn superblock(blocks_lo: u32, blocks_hi: u32, log: u32, incompat: u32) -> [u8; SUPERBLOCK_BYTES] {
        let mut block = [0; SUPERBLOCK_BYTES];
        block[0x04..0x08].copy_from_slice(&blocks_lo.to_le_bytes());
        block[0x18..0x1c].copy_from_slice(&log.to_le_bytes());
        block[0x38..0x3a].copy_from_slice(&MAGIC.to_le_bytes());
        block[0x60..0x64].copy_from_slice(&incompat.to_le_bytes());
        block[0x150..0x154].copy_from_slice(&blocks_hi.to_le_bytes());
        block
    }

    #[test]
    fn a_capacity_is_blocks_times_block_size() {
        assert_eq!(parse_capacity(&superblock(262_144, 0, 2, 0)).map(NonZeroU64::get), Some(1 << 30));
        // The high word counts only with the 64-bit feature.
        assert_eq!(parse_capacity(&superblock(0, 1, 2, 0)), None);
        assert_eq!(
            parse_capacity(&superblock(0, 1, 2, INCOMPAT_64BIT)).map(NonZeroU64::get),
            Some((1 << 32) * 4096)
        );
        let mut not_ext4 = superblock(1, 0, 2, 0);
        not_ext4[0x38] = 0;
        assert_eq!(parse_capacity(&not_ext4), None);
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use std::process::Command;

        use super::super::*;
        use crate::tools::Tools;

        fn output(program: &str, args: &[&OsStr]) -> std::process::Output {
            Command::new(program).args(args).output().expect(program)
        }

        #[tokio::test]
        async fn the_superblock_capacity_matches_dumpe2fs() {
            let tools = Tools::on_path();
            let directory = tempfile::tempdir().unwrap();
            let image = directory.path().join("system.ext4");
            make_system(&tools, &image, NonZeroU64::new(48 << 20).unwrap()).await.unwrap();
            let printed = output("dumpe2fs", &[OsStr::new("-h"), image.as_os_str()]);
            let printed = String::from_utf8_lossy(&printed.stdout);
            let field = |name: &str| -> u64 {
                printed
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .map(|value| value.trim().parse().unwrap())
                    .unwrap_or_else(|| panic!("dumpe2fs printed no {name}"))
            };
            let expected = field("Block count:") * field("Block size:");
            let path = image.clone();
            let read = blocking::run(move |off| capacity(off, &path)).await.unwrap();
            assert_eq!(read.get(), expected);
            assert_eq!(expected, 48 << 20);
        }

        #[tokio::test]
        async fn recovery_completes_an_interrupted_growth() {
            let tools = Tools::on_path();
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("mkhome");
            std::fs::create_dir_all(root.join("demi/work")).unwrap();
            std::fs::write(root.join("demi/work/a.txt"), "alpha\n").unwrap();
            let image = directory.path().join("home.ext4");
            let nominal = 64 << 20;
            make_home(&tools, &root, &image, NonZeroU64::new(nominal).unwrap()).await.unwrap();
            assert_eq!(std::fs::metadata(&image).unwrap().len(), nominal);
            let listed = output(
                "debugfs",
                &[OsStr::new("-R"), OsStr::new("cat /demi/work/a.txt"), image.as_os_str()],
            );
            assert_eq!(String::from_utf8_lossy(&listed.stdout), "alpha\n");

            std::fs::File::options()
                .write(true)
                .open(&image)
                .unwrap()
                .set_len(nominal * 2)
                .unwrap();
            assert_eq!(recover(&tools, &image).await.unwrap().get(), nominal * 2);
            // The filesystem now fills its file and is consistent.
            assert!(output("e2fsck", &[OsStr::new("-fn"), image.as_os_str()]).status.success());
        }
    }
}
