//! Copying a machine image (`managed-hosts.md` § Images): a reflink clone
//! where the filesystem supports one, and otherwise a copy of only the
//! ranges that hold data, skipping holes and all-zero blocks, so unused
//! capacity never becomes allocated host storage. It runs in the manager's
//! process, so the checkpoint's frozen window starts no program.

use std::{
    io,
    os::unix::fs::FileExt,
    path::Path,
};

use fs_err::os::unix::fs::OpenOptionsExt;
use rustix::{fs::SeekFrom, io::Errno};

use crate::blocking::OffLoop;

/// The granularity of the zero-block check: a filesystem block.
const BLOCK: usize = 4096;

/// How much of a data range is read at once.
const CHUNK: usize = 1 << 20;

/// Copies `source` to the new file `destination`, mode 0600, keeping its
/// length and every byte it holds.
pub fn clone_sparse(_: &OffLoop, source: &Path, destination: &Path) -> io::Result<()> {
    let from = fs_err::File::open(source)?;
    let to = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    match rustix::fs::ioctl_ficlone(to.file(), from.file()) {
        Ok(()) => return Ok(()),
        // The filesystem cannot share extents between these files.
        Err(Errno::OPNOTSUPP | Errno::XDEV | Errno::INVAL | Errno::NOTTY | Errno::NOSYS) => {}
        Err(error) => return Err(crate::linux::failed("cloning to", destination, error)),
    }
    let length = from.metadata()?.len();
    copy_data(&from, &to, length)?;
    to.set_len(length)
}

/// Copies each data range of `from`, leaving holes and all-zero blocks
/// unwritten.
fn copy_data(from: &fs_err::File, to: &fs_err::File, length: u64) -> io::Result<()> {
    let mut buffer = vec![0; CHUNK];
    let mut offset = 0;
    while offset < length {
        let data = match rustix::fs::seek(from.file(), SeekFrom::Data(offset)) {
            Ok(data) => data,
            // No data after `offset`: the rest is a hole.
            Err(Errno::NXIO) => break,
            Err(error) => return Err(error.into()),
        };
        let hole = rustix::fs::seek(from.file(), SeekFrom::Hole(data))?;
        let mut position = data;
        while position < hole {
            let count = usize::try_from(hole - position).map_or(CHUNK, |left| left.min(CHUNK));
            let chunk = &mut buffer[..count];
            from.file().read_exact_at(chunk, position)?;
            write_nonzero(to, chunk, position)?;
            position += count as u64;
        }
        offset = hole;
    }
    Ok(())
}

/// Writes the runs of `chunk`'s blocks that are not all zero.
fn write_nonzero(to: &fs_err::File, chunk: &[u8], position: u64) -> io::Result<()> {
    let mut run: Option<usize> = None;
    for (index, block) in chunk.chunks(BLOCK).enumerate() {
        let start = index * BLOCK;
        let zero = block.iter().all(|byte| *byte == 0);
        match (zero, run) {
            (false, None) => run = Some(start),
            (true, Some(first)) => {
                to.file().write_all_at(&chunk[first..start], position + first as u64)?;
                run = None;
            }
            _ => {}
        }
    }
    if let Some(first) = run {
        to.file().write_all_at(&chunk[first..], position + first as u64)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn a_sparse_copy_keeps_holes_and_the_data_at_both_ends() {
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("copy");
        let bytes: u64 = 64 << 20;
        let file = std::fs::File::create(&source).unwrap();
        file.write_all_at(b"head", 0).unwrap();
        // A written block of zeros is copied as a hole too.
        file.write_all_at(&vec![0; 1 << 20], 8 << 20).unwrap();
        file.write_all_at(b"tail", bytes - 4).unwrap();
        drop(file);
        clone_sparse(&off, &source, &destination).unwrap();
        let metadata = std::fs::metadata(&destination).unwrap();
        assert_eq!(metadata.len(), bytes);
        assert!(metadata.blocks() * 512 < 1 << 20, "{} blocks", metadata.blocks());
        let copy = std::fs::File::open(&destination).unwrap();
        for (position, expected) in [(0, b"head"), (bytes - 4, b"tail")] {
            let mut read = [0; 4];
            copy.read_exact_at(&mut read, position).unwrap();
            assert_eq!(&read, expected);
        }
        assert!(clone_sparse(&off, &source, &destination).is_err(), "the copy is new");
    }

    /// Runs `program` with `args` on this thread's namespaces.
    fn run(program: &str, args: &[&std::ffi::OsStr]) {
        let output = std::process::Command::new(program).args(args).output().expect(program);
        assert!(output.status.success(), "{program}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn sha256(path: &Path) -> String {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        std::io::copy(&mut std::fs::File::open(path).unwrap(), &mut hash).unwrap();
        format!("{:x}", hash.finalize())
    }

    /// The design's acceptance of the in-process copy: on each filesystem it
    /// produces the source's bytes and allocates no more blocks than
    /// `cp --reflink=auto --sparse=always`.
    #[test]
    #[ignore = "needs root: run the Linux suite with --ignored as root"]
    fn a_copy_matches_cp_on_xfs_btrfs_and_ext4() {
        use crate::linux::{loopdev, mount};
        crate::linux::testing::isolate();
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        // A sparse ext4 image with files, a written run of zeros and data at
        // both ends, like a working image.
        let content = directory.path().join("content");
        std::fs::create_dir(&content).unwrap();
        for index in 0..8u8 {
            std::fs::write(content.join(format!("file-{index}")), vec![index + 1; 300_000 + usize::from(index) * 4096]).unwrap();
        }
        let source = directory.path().join("source.ext4");
        run("mke2fs", &["-q".as_ref(), "-t".as_ref(), "ext4".as_ref(), "-d".as_ref(), content.as_os_str(), source.as_os_str(), "96m".as_ref()]);
        let file = std::fs::OpenOptions::new().write(true).open(&source).unwrap();
        file.write_all_at(&vec![0; 2 << 20], 40 << 20).unwrap();
        file.write_all_at(b"tail", (96 << 20) - 4).unwrap();
        drop(file);
        for (filesystem, mkfs) in [("xfs", "mkfs.xfs"), ("btrfs", "mkfs.btrfs"), ("ext4", "mkfs.ext4")] {
            let image = directory.path().join(format!("{filesystem}.img"));
            std::fs::File::create(&image).unwrap().set_len(512 << 20).unwrap();
            let device = loopdev::attach(&off, &image).unwrap();
            let quiet: &[&str] = match filesystem {
                "ext4" => &["-q", "-F"],
                _ => &["-q", "-f"],
            };
            let mut args: Vec<&std::ffi::OsStr> = quiet.iter().map(|arg| arg.as_ref()).collect();
            let path = device.path();
            args.push(path.as_os_str());
            run(mkfs, &args);
            let target = directory.path().join(filesystem);
            std::fs::create_dir(&target).unwrap();
            rustix::mount::mount(&path, &target, filesystem, rustix::mount::MountFlags::empty(), None::<&std::ffi::CStr>)
                .unwrap();
            drop(device);
            let inside = target.join("source.ext4");
            run("cp", &["--sparse=always".as_ref(), source.as_os_str(), inside.as_os_str()]);
            let ours = target.join("ours.ext4");
            let theirs = target.join("theirs.ext4");
            clone_sparse(&off, &inside, &ours).unwrap();
            run("cp", &["--reflink=auto".as_ref(), "--sparse=always".as_ref(), inside.as_os_str(), theirs.as_os_str()]);
            run("sync", &["-f".as_ref(), target.as_os_str()]);
            let blocks = |path: &Path| std::fs::metadata(path).unwrap().blocks();
            assert_eq!(sha256(&ours), sha256(&inside), "{filesystem}: content");
            assert_eq!(std::fs::metadata(&ours).unwrap().len(), 96 << 20, "{filesystem}: length");
            assert!(
                blocks(&ours) <= blocks(&theirs),
                "{filesystem}: {} blocks against cp's {}",
                blocks(&ours),
                blocks(&theirs)
            );
            eprintln!("{filesystem}: ours {} blocks, cp {} blocks, source {}", blocks(&ours), blocks(&theirs), blocks(&inside));
            mount::unmount(&off, &target).unwrap();
            // Unmounting detaches the auto-clear loop device.
            assert!(loop_detaches(&image), "{filesystem}: the loop device stays attached");
        }
    }

    /// Whether a loop device still has `image` as its backing file.
    pub(crate) fn loop_attached(image: &Path) -> bool {
        let image = std::fs::canonicalize(image).unwrap();
        std::fs::read_dir("/sys/block").unwrap().any(|entry| {
            let backing = entry.unwrap().path().join("loop/backing_file");
            std::fs::read_to_string(backing).is_ok_and(|file| Path::new(file.trim()) == image)
        })
    }

    /// Whether the loop device of `image` detaches within five seconds: the
    /// kernel runs an auto-clear detach after the last user closes it.
    pub(crate) fn loop_detaches(image: &Path) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while loop_attached(image) {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        true
    }
}
