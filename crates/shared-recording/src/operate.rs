//! What `record` does once a program has found the recording, the same for
//! every program: the answers `start`, `stop`, `frames` and `export` give,
//! as the result a script reads and the text a person reads.
//!
//! Where an output goes, which a program resolves against its caller's
//! directory, is the program's.

use std::path::{Path, PathBuf};

use demi_shared_recording_protocol::{
    ExportedFrame, FrameSet, Input, InputRow, RecordAction, RecordResult, RecordingId, Region, text,
};

use crate::{
    Crop, Error, Export, Library, Live, sheet,
    stored,
    video::{self, Part},
};

/// What `frames` and `export` ask for.
pub struct Reading<'a> {
    /// The command's path, such as `demi mobile record`, which the text
    /// names the commands to go on with by.
    pub command: &'a str,
    pub frames: Option<&'a FrameSet>,
    pub whole: bool,
    pub region: Option<Region>,
    pub mp4: bool,
    /// Whether the export leaves the inputs off the frames.
    pub plain: bool,
    /// Where the images go, or with `mp4` the file: a path the program
    /// resolved; none for the default.
    pub output: Option<&'a Path>,
    pub overwrite: bool,
    /// Whether an MP4 plays the waits faster.
    pub fast_waits: bool,
    /// Whether the export is one PNG of the whole recordings.
    pub sheet: bool,
    /// The time between a sheet's tiles; the default when none.
    pub every: Option<std::time::Duration>,
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
                inputs: Some(listing.inputs),
                ..RecordResult::of(listing.recording)
            },
        ))
    }

    /// Lists the recordings of `target`, newest first.
    pub async fn listing(&self, target: &str) -> Result<Answer, Error> {
        let recordings = self.list(target).await?;
        let newest = recordings
            .first()
            .cloned()
            .ok_or_else(|| Error::BadRequest(format!("{target} has no recording this conversation keeps")))?;
        Ok(answer(
            RecordAction::List,
            "",
            RecordResult {
                recordings: Some(recordings),
                ..RecordResult::of(newest)
            },
        ))
    }

    /// Keeps `text` as a note of `id` at `at` of its frames' time, or now
    /// while it records.
    pub async fn note(&self, id: &RecordingId, text: &str, at: Option<std::time::Duration>) -> Result<Answer, Error> {
        let stored = self.stored(id).await?;
        let info = stored.info()?;
        let at = at.map(|at| u64::try_from(at.as_micros()).unwrap_or(u64::MAX));
        if let Some(at) = at
            && at > info.duration_ms * 1000
        {
            return Err(Error::BadRequest(format!(
                "{id} {} {} s; --at {} s is after that",
                if info.running { "has run" } else { "ran" },
                text::seconds(info.duration_ms * 1000),
                text::seconds(at)
            )));
        }
        let time = match (&stored.live, at) {
            (Some(live), at) => live.sink().note(text.to_owned(), at).await?,
            (None, _) if info.running || stored::written(stored.directory()) => {
                return Err(Error::BadRequest(format!(
                    "{id} is recorded by another program; set its note with the command that started it"
                )));
            }
            (None, None) => {
                return Err(Error::BadRequest(format!(
                    "{id} ended; give the second of it the note shows from, such as --at 2.5"
                )));
            }
            (None, Some(at)) => {
                stored::append_input(
                    stored.directory(),
                    &InputRow {
                        time_us: at,
                        input: Input::Note { text: text.to_owned() },
                    },
                )?;
                at
            }
        };
        Ok(answer(
            RecordAction::Note,
            "",
            RecordResult {
                inputs: Some(vec![InputRow {
                    time_us: time,
                    input: Input::Note { text: text.to_owned() },
                }]),
                ..RecordResult::of(info)
            },
        ))
    }

    /// Writes the frames of the recording `ids` names as images, or the
    /// video or the sheet of every recording it names, one after another.
    pub async fn export(&self, ids: &[RecordingId], reading: &Reading<'_>) -> Result<Answer, Error> {
        let mut stored: Vec<crate::Stored> = Vec::with_capacity(ids.len());
        for id in ids {
            stored.push(self.stored(id).await?);
        }
        let infos = stored.iter().map(crate::Stored::info).collect::<Result<Vec<_>, _>>()?;
        let info = infos.first().cloned().ok_or_else(|| Error::BadRequest("export names no recording".into()))?;
        let joined = (infos.len() > 1).then(|| infos.clone());
        if reading.mp4 || reading.sheet {
            let extension = if reading.mp4 { "mp4" } else { "png" };
            let target = match reading.output {
                Some(path) => {
                    if path.exists() && !reading.overwrite {
                        return Err(Error::Exists(path.to_owned()));
                    }
                    path.to_owned()
                }
                None if reading.mp4 => stored[0].directory().join(format!("export-{}.mp4", unique())),
                None => {
                    let ids = ids.iter().map(RecordingId::as_str).collect::<Vec<_>>().join("-");
                    reading.scratch.join(format!("sheet-{ids}.{extension}"))
                }
            };
            let parts = stored.iter().map(Part::of).collect::<Result<Vec<_>, _>>()?;
            let parts = std::sync::Arc::new(parts);
            if reading.sheet {
                let load = self.codec.library().await.map_err(|error| Error::Unavailable(error.to_string()))?;
                let (written, plain) = (target.clone(), reading.plain);
                let every = reading.every.map(|every| u64::try_from(every.as_micros()).unwrap_or(u64::MAX));
                if let Some(parent) = written.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, &error))?;
                }
                let unshown = tokio::task::spawn_blocking(move || {
                    sheet::write(&written, &sheet::Sheet { parts: &parts, every, plain, load: &load })
                })
                .await
                .map_err(|error| Error::Failed(format!("the export stopped: {error}")))??;
                return Ok(answer(
                    RecordAction::Export,
                    reading.command,
                    RecordResult {
                        sheet: Some(target.display().to_string()),
                        recordings: joined,
                        unshown: (!unshown.is_empty()).then_some(unshown),
                        ..RecordResult::of(info)
                    },
                ));
            }
            // Drawing needs the coder, and so do several recordings and a
            // screen that changed size, which are coded again; a plain video of
            // one recording does not, and goes without fetching it. A Host that
            // cannot load it gives a video it can plain, and says why.
            let needed = !reading.plain || parts.len() > 1 || video::resized(&parts);
            let (load, unloaded) = match needed {
                false => (None, None),
                true => match self.codec.library().await {
                    Ok(load) => (Some(load), None),
                    Err(error) => (None, Some(error.to_string())),
                },
            };
            let (fast_waits, drawn) = (reading.fast_waits, !reading.plain);
            let write = |load: Option<demi_shared_openh264::Load>| {
                let (parts, written) = (parts.clone(), target.clone());
                tokio::task::spawn_blocking(move || {
                    video::write(&written, &video::Video { parts: &parts, load: load.as_ref(), drawn, fast_waits })
                })
            };
            let mut undrawn = unloaded.clone().filter(|_| drawn);
            let written = match write(load.clone()).await.map_err(|error| Error::Failed(format!("the export stopped: {error}")))? {
                Err(Error::Unavailable(why)) if load.is_some() => {
                    undrawn = Some(why);
                    write(None).await.map_err(|error| Error::Failed(format!("the export stopped: {error}")))??
                }
                Err(Error::Unavailable(why)) => {
                    return Err(Error::Unavailable(format!("{why}, and this Host cannot load it: {}", unloaded.unwrap_or(why.clone()))));
                }
                written => written?,
            };
            let ran: u64 = infos.iter().map(|info| info.duration_ms).sum::<u64>() * 1000;
            let plays = (fast_waits && written.plays < ran).then_some(written.plays);
            let names = ids.iter().map(RecordingId::as_str).collect::<Vec<_>>().join(", ");
            let mut about = match plays {
                Some(plays) => format!("{names} · {} · {} s MP4, played in {} s", info.target, text::seconds(ran), text::seconds(plays)),
                None => format!("{names} · {} · {} s MP4", info.target, text::seconds(ran)),
            };
            if let Some(why) = &undrawn {
                about.push_str(&format!(" · inputs not drawn: {why}"));
            }
            for unshown in &written.unshown {
                about.push_str(&format!(" · {unshown}"));
            }
            return Ok(match reading.output {
                Some(path) => answer(
                    RecordAction::Export,
                    reading.command,
                    RecordResult {
                        path: Some(path.display().to_string()),
                        undrawn,
                        recordings: joined,
                        unshown: (!written.unshown.is_empty()).then_some(written.unshown),
                        plays_ms: plays.map(|plays| plays / 1000),
                        ..RecordResult::of(info)
                    },
                ),
                None => Answer::Video { file: target, about },
            });
        }
        if stored.len() != 1 {
            return Err(Error::BadRequest(
                "images are of one recording; several play one after another with --mp4, or show on one --sheet".into(),
            ));
        }
        let stored = stored.pop().expect("one recording");
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
        let plain = reading.plain;
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
                plain,
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
