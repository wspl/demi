//! What `record` does once a program has found the recording, the same for
//! every program: the answers `start`, `stop`, `frames` and `export` give,
//! as the result a script reads and the text a person reads.
//!
//! Where an output goes, which a program resolves against its caller's
//! directory, is the program's.

use std::path::{Path, PathBuf};

use demi_shared_recording_protocol::{
    ExportedFrame, FrameSet, RecordAction, RecordResult, RecordingId, Region, text,
};

use crate::{Crop, Error, Export, Library, Live};

/// What `frames` and `export` ask for.
pub struct Reading<'a> {
    /// The command's path, such as `demi mobile record`, which the text
    /// names the commands to go on with by.
    pub command: &'a str,
    pub frames: Option<&'a FrameSet>,
    pub whole: bool,
    pub region: Option<Region>,
    pub mp4: bool,
    /// Where the images go, or with `mp4` the file: a path the program
    /// resolved; none for the default.
    pub output: Option<&'a Path>,
    pub overwrite: bool,
    /// The directory an export without an output makes its own in: the
    /// caller's scratch directory.
    pub scratch: &'a Path,
}

/// What `record` answers.
pub enum Answer {
    Result { result: RecordResult, text: String },
    /// The recording's video, written to a file of its own, which the
    /// program streams and removes.
    Video { file: PathBuf, about: String },
}

/// What `start` answers for `live`.
pub fn started(command: &str, live: &Live) -> Result<Answer, Error> {
    Ok(answer(RecordAction::Start, command, RecordResult::of(live.info()?)))
}

impl Library {
    /// Stops the recording `id`, or answers for one that stopped already.
    pub async fn stop(&self, id: &RecordingId) -> Result<Answer, Error> {
        let info = match self.live(id) {
            Some(live) => live.stop(demi_shared_recording_protocol::EndReason::Stopped).await?,
            None => self.stored(id).await?.info()?,
        };
        Ok(answer(RecordAction::Stop, "", RecordResult::of(info)))
    }

    /// Lists the frames of `id`.
    pub async fn frames(&self, id: &RecordingId, reading: &Reading<'_>) -> Result<Answer, Error> {
        let stored = self.stored(id).await?;
        let listing = stored.frames(reading.frames)?;
        Ok(answer(
            RecordAction::Frames,
            reading.command,
            RecordResult {
                frames: Some(listing.frames),
                gaps: Some(listing.gaps),
                ..RecordResult::of(listing.recording)
            },
        ))
    }

    /// Writes the frames of `id` as images, or its video as an MP4.
    pub async fn export(&self, id: &RecordingId, reading: &Reading<'_>) -> Result<Answer, Error> {
        let stored = self.stored(id).await?;
        let info = stored.info()?;
        if reading.mp4 {
            let target = match reading.output {
                Some(path) => {
                    if path.exists() && !reading.overwrite {
                        return Err(Error::Exists(path.to_owned()));
                    }
                    path.to_owned()
                }
                None => stored.directory().join(format!("export-{}.mp4", unique())),
            };
            let written = target.clone();
            let ran = tokio::task::spawn_blocking(move || stored.write_mp4(&written))
                .await
                .map_err(|error| Error::Failed(format!("the export stopped: {error}")))??;
            let about = format!("{} · {} · {:.1} s MP4", info.id, info.target, ran.as_secs_f64());
            return Ok(match reading.output {
                Some(path) => answer(
                    RecordAction::Export,
                    reading.command,
                    RecordResult {
                        path: Some(path.display().to_string()),
                        ..RecordResult::of(info)
                    },
                ),
                None => Answer::Video { file: target, about },
            });
        }
        let directory = match reading.output {
            Some(path) => path.to_owned(),
            None => reading.scratch.join(format!("recording-{}", info.id)),
        };
        let crop = match (reading.region, reading.whole) {
            (Some(region), _) => Crop::Region(region.0),
            (None, true) => Crop::Whole,
            (None, false) => Crop::Changed,
        };
        let frames = reading.frames.cloned();
        let overwrite = reading.overwrite || reading.output.is_none();
        let load = self.codec.library().await.map_err(|error| Error::Unavailable(error.to_string()))?;
        // A directory the export chose itself is replaced by a later export of
        // the same frames; one the caller named refuses files that are there.
        let exported = tokio::task::spawn_blocking(move || {
            stored.export(&Export {
                frames: frames.as_ref(),
                crop,
                directory: &directory,
                overwrite,
                load: &load,
            })
        })
        .await
        .map_err(|error| Error::Failed(format!("the export stopped: {error}")))??;
        Ok(answer(
            RecordAction::Export,
            reading.command,
            RecordResult {
                images: Some(
                    exported
                        .images
                        .iter()
                        .map(|(frame, path)| ExportedFrame {
                            frame: *frame,
                            path: path.display().to_string(),
                        })
                        .collect(),
                ),
                area: Some(exported.area),
                ..RecordResult::of(info)
            },
        ))
    }
}

fn answer(action: RecordAction, command: &str, result: RecordResult) -> Answer {
    Answer::Result {
        text: text::record(action, command, &result),
        result,
    }
}

fn unique() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
