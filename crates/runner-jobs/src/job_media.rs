//! A job's media (`runner.md` § Pipes and output): the images and videos its
//! declared commands return to the job, numbered from 1 in the order they
//! reach the runner and kept in `media/<n>` of the job's directory within
//! the job's bounds, each announced to the backend as `job_medium`; and the
//! medium a command whose stdout goes elsewhere returned, held in the same
//! directory until the command ends (`commands.md` § Return media).

use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{Mutex, PoisonError},
};

use bytes::Bytes;
use demi_runner_protocol::wire;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// The directory of a job's directory that holds its media.
pub const MEDIA_DIRECTORY: &str = "media";

/// The most media a job keeps.
pub const JOB_MEDIA: u32 = 32;

/// The most bytes of media a job keeps.
pub const JOB_MEDIA_BYTES: u64 = 64 * 1024 * 1024;

/// What stands in a command's stdout for a medium beyond the job's bounds.
pub const NOT_KEPT_LINE: &str = "[medium not kept: a job keeps at most 32 media and 64 MiB]";

/// The line a command's stdout holds where it returned a medium the job
/// keeps.
pub fn medium_line(number: u32, media_type: &str, size: u64) -> String {
    format!("[medium {number}: {media_type}, {size} bytes]")
}

/// One job's media.
pub struct JobMedia {
    job_id: String,
    /// `media/` in the job's directory, made with the first medium.
    directory: PathBuf,
    /// Where `job_medium` goes: the job's output channel, so each one
    /// precedes the job's exit.
    output: mpsc::Sender<wire::Frame>,
    kept: Mutex<Kept>,
}

/// How many media the job keeps, and their bytes.
#[derive(Default)]
struct Kept {
    count: u32,
    bytes: u64,
}

/// A medium held for a command whose stdout goes elsewhere; its file goes
/// when it drops.
pub struct HeldMedium(tempfile::NamedTempFile);

impl JobMedia {
    pub fn new(job_id: String, directory: PathBuf, output: mpsc::Sender<wire::Frame>) -> Self {
        Self {
            job_id,
            directory,
            output,
            kept: Mutex::default(),
        }
    }

    /// Keeps `bytes`, a checked medium of `media_type`, as the job's next
    /// medium, writes it to its file and tells the backend; its number, or
    /// none when it is beyond the job's bounds and nothing of it is kept.
    pub async fn keep(&self, media_type: &str, bytes: Bytes) -> io::Result<Option<u32>> {
        let size = bytes.len() as u64;
        let number = {
            // No section panics while it holds the lock.
            let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            if kept.count >= JOB_MEDIA || kept.bytes + size > JOB_MEDIA_BYTES {
                return Ok(None);
            }
            kept.count += 1;
            kept.bytes += size;
            kept.count
        };
        let directory = self.directory.clone();
        let sha256 = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory)?;
            std::fs::write(directory.join(number.to_string()), &bytes)?;
            Ok::<_, io::Error>(format!("{:x}", Sha256::digest(&bytes)))
        })
        .await
        .map_err(io::Error::other)??;
        let message = wire::encode(&wire::Outbound::JobMedium {
            job_id: self.job_id.clone(),
            number,
            media_type: media_type.to_owned(),
            size,
            sha256,
        })
        .map_err(io::Error::other)?;
        self.output
            .send(message)
            .await
            .map_err(|_| io::Error::other("host connection closed"))?;
        Ok(Some(number))
    }

    /// Holds `bytes` in the job's directory until the command ends.
    pub async fn hold(&self, bytes: Bytes) -> io::Result<HeldMedium> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory)?;
            let mut file = tempfile::Builder::new()
                .prefix(".held-")
                .tempfile_in(&directory)?;
            file.write_all(&bytes)?;
            Ok(HeldMedium(file))
        })
        .await
        .map_err(io::Error::other)?
    }
}

impl HeldMedium {
    /// The held bytes; the file goes.
    pub async fn take(self) -> io::Result<Bytes> {
        tokio::task::spawn_blocking(move || std::fs::read(self.0.path()).map(Bytes::from))
            .await
            .map_err(io::Error::other)?
    }
}
