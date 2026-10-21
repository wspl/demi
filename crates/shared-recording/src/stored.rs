//! A recording's files and how they are read.
//!
//! ```text
//! <directory>/r<number>/
//!   recording.json   what it records, and how it ended once it has
//!   frames.jsonl     one line for each picture or run of frames missed
//!   video.h264       the pictures' H.264, Annex B, one coded picture each
//! ```
//!
//! A source codes only the pictures that differ from the one before, and says
//! where each differs, so each picture of the video is a frame.
//!
//! Everything a listing says of a recording, how many frames it kept, where
//! they changed, how many it missed, is read from `frames.jsonl`: nothing
//! derivable is stored a second time. A line is written, and the video's bytes
//! it names flushed, before the next frame is taken, so a recording is readable
//! while it runs and after its program ended.

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use demi_shared_recording_protocol::{Area, EndReason, FrameRow, FrameSize, Gap, RecordingId, RecordingInfo};
use serde::{Deserialize, Serialize};

use crate::Error;

pub(crate) const META: &str = "recording.json";
pub(crate) const INDEX: &str = "frames.jsonl";
pub(crate) const VIDEO: &str = "video.h264";
/// The file a program holds an exclusive lock on while it writes a
/// recording, which the system releases when the program ends, however it
/// does: whether another program still writes a recording is whether the lock
/// is held.
pub(crate) const WRITING: &str = "writing";

/// Whether a program, this one or another, writes the recording in
/// `directory` now.
pub(crate) fn written(directory: &Path) -> bool {
    let Ok(file) = fs::File::open(directory.join(WRITING)) else {
        return false;
    };
    // A lock taken is let go at once: nothing writes here, and nothing starts
    // writing in a recording that began.
    matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Meta {
    pub target: String,
    pub ended: Option<Ended>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Ended {
    pub reason: EndReason,
    pub note: Option<String>,
    /// How long it ran, from its first frame.
    pub duration_us: u64,
}

/// One frame of the video: a picture that differs from the one before.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Kept {
    /// Microseconds from the first frame.
    pub t: u64,
    /// Where it differs from the frame before.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Its size.
    pub fw: u32,
    pub fh: u32,
    /// Its bytes in the video, and whether it is a key frame.
    pub o: u64,
    pub l: u32,
    pub key: bool,
}

impl Kept {
    pub fn changed(&self) -> Area {
        Area {
            x: self.x,
            y: self.y,
            width: self.w,
            height: self.h,
        }
    }

    pub fn size(&self) -> FrameSize {
        FrameSize {
            width: self.fw,
            height: self.fh,
        }
    }

    /// Whether it differs everywhere, as the first frame and one of another
    /// size do.
    pub fn whole(&self) -> bool {
        (self.x, self.y, self.w, self.h) == (0, 0, self.fw, self.fh)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", deny_unknown_fields)]
pub(crate) enum Entry {
    #[serde(rename = "f")]
    Frame(Kept),
    #[serde(rename = "m")]
    Missed { after: u32, count: u32 },
}

/// What a recording's files hold now.
pub(crate) struct Contents {
    pub meta: Meta,
    /// Every frame of the video, numbered from 1 in this order.
    pub frames: Vec<Kept>,
    pub gaps: Vec<Gap>,
}

impl Contents {
    pub fn read(directory: &Path) -> Result<Self, Error> {
        let meta = fs::read(directory.join(META)).map_err(|error| Error::io(directory, &error))?;
        let meta: Meta = serde_json::from_slice(&meta)
            .map_err(|error| Error::Corrupt(format!("{}: {error}", directory.join(META).display())))?;
        let mut text = String::new();
        fs::File::open(directory.join(INDEX))
            .and_then(|mut file| file.read_to_string(&mut text))
            .map_err(|error| Error::io(directory, &error))?;
        // A line a program killed mid-write left unfinished is no frame.
        let finished = text.rfind('\n').map_or(0, |end| end + 1);
        let (mut frames, mut gaps) = (Vec::new(), Vec::new());
        for line in text[..finished].lines() {
            match serde_json::from_str::<Entry>(line)
                .map_err(|error| Error::Corrupt(format!("{}: {error}", directory.join(INDEX).display())))?
            {
                Entry::Frame(kept) => frames.push(kept),
                Entry::Missed { after, count } => gaps.push(Gap { after, count }),
            }
        }
        Ok(Self { meta, frames, gaps })
    }

    /// The recording as `start` and `stop` tell it. `running` is whether this
    /// program still writes it; a recording with no end that none writes was
    /// interrupted.
    pub fn info(&self, id: &RecordingId, running: bool, ran_us: u64) -> RecordingInfo {
        let (ended, note, duration_us) = match (&self.meta.ended, running) {
            (Some(ended), _) => (Some(ended.reason), ended.note.clone(), ended.duration_us),
            (None, true) => (None, None, ran_us),
            (None, false) => (
                Some(EndReason::Interrupted),
                None,
                self.frames.last().map_or(0, |last| last.t),
            ),
        };
        RecordingInfo {
            id: id.clone(),
            target: self.meta.target.clone(),
            running: ended.is_none(),
            ended,
            note,
            duration_ms: duration_us / 1000,
            frames: self.frames.len() as u64,
            missed: self.gaps.iter().map(|gap| u64::from(gap.count)).sum(),
            area: self
                .frames
                .iter()
                .map(Kept::changed)
                .reduce(Area::union)
                .filter(|_| !self.frames.is_empty()),
            size: self.frames.first().map(Kept::size),
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = FrameRow> + '_ {
        self.frames.iter().enumerate().map(|(index, kept)| FrameRow {
            frame: index as u32 + 1,
            time_us: kept.t,
            changed: kept.changed(),
            whole: kept.whole(),
        })
    }
}

/// The recordings in `directory`, oldest first.
pub(crate) fn listed(directory: &Path) -> Vec<(RecordingId, PathBuf)> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut found: Vec<(RecordingId, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let id = entry.file_name().to_str()?.parse::<RecordingId>().ok()?;
            Some((id, entry.path()))
        })
        .collect();
    found.sort_by_key(|(id, _)| id.number());
    found
}
