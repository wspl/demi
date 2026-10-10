//! A job's media (`runner.md` § Pipes and output): the images, videos and
//! PDF documents `demi file view` hands to the job, whatever its stdout is,
//! numbered from 1 in the order they reach the runner and kept in
//! `media/<n>` of the job's directory within the job's bounds. Each one's
//! line goes into the job's output as it arrives, and each one kept is
//! announced to the backend as `job_medium`
//! (`runtime.md` § What `demi file view` shows).

use std::{
    io,
    path::PathBuf,
    sync::{Mutex, PoisonError},
};

use bytes::Bytes;
use demi_command_protocol::medium_facts;
use demi_runner_protocol::wire;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// The directory of a job's directory that holds its media.
pub const MEDIA_DIRECTORY: &str = "media";

/// The most media a job keeps.
pub const JOB_MEDIA: u32 = 32;

/// The most bytes of media a job keeps.
pub const JOB_MEDIA_BYTES: u64 = 64 * 1024 * 1024;

/// A medium as it reaches the job's output: its line, then the
/// `job_medium` that announces it when the job keeps it.
pub struct Arrival {
    pub line: String,
    pub medium: Option<wire::Frame>,
}

/// One job's media.
pub struct JobMedia {
    job_id: String,
    /// `media/` in the job's directory, made with the first medium.
    directory: PathBuf,
    /// Where each medium's arrival goes: the job's own loop, which writes
    /// its line into the job's output and sends its `job_medium` before the
    /// job's exit.
    arrivals: mpsc::UnboundedSender<Arrival>,
    counts: Mutex<Counts>,
}

/// How many media reached the job, and how many it keeps with their bytes.
#[derive(Default)]
struct Counts {
    arrived: u32,
    kept: u32,
    bytes: u64,
}

impl JobMedia {
    pub fn new(
        job_id: String,
        directory: PathBuf,
        arrivals: mpsc::UnboundedSender<Arrival>,
    ) -> Self {
        Self {
            job_id,
            directory,
            arrivals,
            counts: Mutex::default(),
        }
    }

    /// Takes `bytes`, a checked medium of `media_type`, as the job's next
    /// medium: within the job's bounds it writes it to its file, and its
    /// line, and the announcement of one it keeps, go to the job's output.
    pub async fn keep(&self, media_type: &str, bytes: Bytes) -> io::Result<()> {
        let size = bytes.len() as u64;
        let (number, kept) = {
            // No section panics while it holds the lock.
            let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            counts.arrived += 1;
            let kept = counts.kept < JOB_MEDIA && counts.bytes + size <= JOB_MEDIA_BYTES;
            if kept {
                counts.kept += 1;
                counts.bytes += size;
            }
            (counts.arrived, kept)
        };
        let kind = kind(media_type);
        if !kept {
            return self.arrive(Arrival {
                line: format!(
                    "[{kind} {number}: not kept: a job keeps at most {JOB_MEDIA} media and 64 MiB]"
                ),
                medium: None,
            });
        }
        let directory = self.directory.clone();
        let owned_type = media_type.to_owned();
        let (sha256, header) = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory)?;
            std::fs::write(directory.join(number.to_string()), &bytes)?;
            let header = medium_facts(&bytes, &owned_type);
            Ok::<_, io::Error>((format!("{:x}", Sha256::digest(&bytes)), header))
        })
        .await
        .map_err(io::Error::other)??;
        let mut facts = vec![media_type.to_owned()];
        if let Some(duration) = header.duration_ms {
            facts.push(format!("{:.1} s", duration as f64 / 1000.0));
        }
        if let Some((width, height)) = header.size {
            facts.push(format!("{width} × {height} px"));
        }
        facts.push(format!("{size} bytes"));
        let medium = wire::encode(&wire::Outbound::JobMedium {
            job_id: self.job_id.clone(),
            number,
            media_type: media_type.to_owned(),
            size,
            sha256,
        })
        .map_err(io::Error::other)?;
        self.arrive(Arrival {
            line: format!("[{kind} {number}: {}]", facts.join(", ")),
            medium: Some(medium),
        })
    }

    fn arrive(&self, arrival: Arrival) -> io::Result<()> {
        self.arrivals
            .send(arrival)
            .map_err(|_| io::Error::other("the job has ended"))
    }
}

/// The word a medium's line names its kind by.
fn kind(media_type: &str) -> &'static str {
    match media_type.split_once('/').map(|(kind, _)| kind) {
        Some("image") => "image",
        Some("video") => "video",
        _ => "document",
    }
}
