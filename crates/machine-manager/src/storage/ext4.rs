//! A device's ext4 images: making the two filesystems, reading a capacity
//! from the superblock, enlarging an unmounted one to the capacity a wake
//! names (`crates/backend-cloud/src/lib.rs`), and checking an unmounted
//! image before it is published.

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
        u32::from_le_bytes(
            superblock[offset..offset + 4]
                .try_into()
                .expect("four bytes"),
        )
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
pub async fn make_home(
    tools: &Tools,
    root: &Path,
    image: &Path,
    bytes: NonZeroU64,
) -> Result<(), Ext4Error> {
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

/// Checks an unmounted image, then syncs it and returns its capacity. A
/// check runs as long as it needs.
pub async fn recover(tools: &Tools, image: &Path) -> Result<NonZeroU64, Ext4Error> {
    check(tools, image, "-p").await?;
    let path = image.to_owned();
    blocking::run(move |off| {
        sync(off, &path)?;
        capacity(off, &path)
    })
    .await
}

/// Enlarges the unmounted image to `bytes`: extends its sparse file, then
/// grows its filesystem to fill it, after the full check `resize2fs`
/// requires; syncs it and returns the capacity its superblock reports. A
/// check or a resize runs as long as it needs.
pub async fn enlarge(tools: &Tools, image: &Path, bytes: NonZeroU64) -> Result<NonZeroU64, Ext4Error> {
    let path = image.to_owned();
    blocking::run(move |_| -> io::Result<()> {
        let file = fs_err::OpenOptions::new().write(true).open(&path)?;
        if file.metadata()?.len() < bytes.get() {
            file.set_len(bytes.get())?;
        }
        Ok(())
    })
    .await?;
    check(tools, image, "-pf").await?;
    tools.run(Tool::Resize2fs, [image], None).await?;
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

    fn superblock(
        blocks_lo: u32,
        blocks_hi: u32,
        log: u32,
        incompat: u32,
    ) -> [u8; SUPERBLOCK_BYTES] {
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
        assert_eq!(
            parse_capacity(&superblock(262_144, 0, 2, 0)).map(NonZeroU64::get),
            Some(1 << 30)
        );
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
            make_system(&tools, &image, NonZeroU64::new(48 << 20).unwrap())
                .await
                .unwrap();
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
            let read = blocking::run(move |off| capacity(off, &path))
                .await
                .unwrap();
            assert_eq!(read.get(), expected);
            assert_eq!(expected, 48 << 20);
        }
    }
}
