//! A job's media: the images, videos and PDF documents `demi file view` hands to the
//! job, whatever its stdout is, kept in `media/<n>` of the job's directory, numbered
//! from 1 in arrival order, within [`JOB_MEDIA`] media and [`JOB_MEDIA_BYTES`]; one
//! beyond the bounds is read to its end and dropped, and its line says it was not
//! kept. Each medium's line goes into the job's output as it arrives, and the
//! command that returned it ends only once the line is written ([`Arrival`]), so
//! nothing the job prints afterwards overtakes it. A kept medium is announced as
//! `job_medium` once written, so the backend knows each medium even when the
//! connection is lost before the job ends; every `job_medium` of a job precedes its
//! `job_exit`.

use std::{
    io,
    path::PathBuf,
    sync::{Mutex, PoisonError},
};

use bytes::Bytes;
use demi_command_protocol::MediumFacts;
use demi_runner_protocol::wire;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

/// The directory of a job's directory that holds its media.
pub const MEDIA_DIRECTORY: &str = "media";

/// The most media a job keeps.
pub const JOB_MEDIA: u32 = 32;

/// The most bytes of media a job keeps.
pub const JOB_MEDIA_BYTES: u64 = 64 * 1024 * 1024;

/// A medium as it reaches the job's output: its line, then the
/// `job_medium` that announces it when the job keeps it. `written` is
/// answered once its line is in the job's output: the command that returned
/// the medium ends only then, so nothing the job prints after the command
/// overtakes the line.
pub struct Arrival {
    pub line: String,
    pub medium: Option<wire::Frame>,
    pub written: oneshot::Sender<()>,
}

/// One job's media.
pub struct JobMedia {
    job_id: String,
    /// `media/` in the job's directory, made with the first medium.
    directory: PathBuf,
    /// Where each medium's arrival goes: the job's own loop, which writes
    /// its line into the job's output, answers its `written`, and sends its
    /// `job_medium` before the job's exit.
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

    /// Takes `bytes`, a checked medium that `facts` describe, as the job's
    /// next medium: within the job's bounds it writes it to its file, and
    /// its line, from the facts, and the announcement of one it keeps, which
    /// carries them on, go to the job's output. It returns once the line is
    /// in the job's output.
    pub async fn keep(&self, facts: MediumFacts, bytes: Bytes) -> io::Result<()> {
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
        let kind = kind(&facts.media_type);
        if !kept {
            return self
                .arrive(
                    format!(
                        "[{kind} {number}: not kept: a job keeps at most {JOB_MEDIA} media and 64 MiB]"
                    ),
                    None,
                )
                .await;
        }
        let directory = self.directory.clone();
        let sha256 = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory)?;
            std::fs::write(directory.join(number.to_string()), &bytes)?;
            Ok::<_, io::Error>(format!("{:x}", Sha256::digest(&bytes)))
        })
        .await
        .map_err(io::Error::other)??;
        let line = format!("[{kind} {number}: {}]", line_facts(number, &facts, size));
        let medium = wire::encode(&wire::Outbound::JobMedium {
            job_id: self.job_id.clone(),
            number,
            media_type: facts.media_type,
            size,
            sha256,
            name: facts.name,
            width: facts.width,
            height: facts.height,
            duration_ms: facts.duration_ms,
        })
        .map_err(io::Error::other)?;
        self.arrive(line, Some(medium)).await
    }

    /// Hands a medium's line and announcement to the job, and waits until
    /// the line is in its output.
    async fn arrive(&self, line: String, medium: Option<wire::Frame>) -> io::Result<()> {
        let ended = || io::Error::other("the job has ended");
        let (written, done) = oneshot::channel();
        self.arrivals
            .send(Arrival {
                line,
                medium,
                written,
            })
            .map_err(|_| ended())?;
        done.await.map_err(|_| ended())
    }
}

/// What a medium's line says after its kind and number: a document's name,
/// `document-<n>.pdf` for one with none, the media type, a video's length,
/// the size in pixels, and the bytes.
fn line_facts(number: u32, facts: &MediumFacts, size: u64) -> String {
    let mut parts = Vec::new();
    if kind(&facts.media_type) == "document" {
        parts.push(
            facts
                .name
                .clone()
                .unwrap_or_else(|| format!("document-{number}.pdf")),
        );
    }
    parts.push(facts.media_type.clone());
    if let Some(duration) = facts.duration_ms {
        parts.push(format!("{:.1} s", duration as f64 / 1000.0));
    }
    if let (Some(width), Some(height)) = (facts.width, facts.height) {
        parts.push(format!("{width} × {height} px"));
    }
    parts.push(format!("{size} bytes"));
    parts.join(", ")
}

/// The word a medium's line names its kind by.
fn kind(media_type: &str) -> &'static str {
    match media_type.split_once('/').map(|(kind, _)| kind) {
        Some("image") => "image",
        Some("video") => "video",
        _ => "document",
    }
}
