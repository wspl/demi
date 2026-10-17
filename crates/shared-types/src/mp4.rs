//! What an MP4 file holds, read from its header: the one reader of a
//! generated video's file, which `demi video` checks the video against
//! what was asked with and the usage ledger takes its seconds from
//! (`videos.md` § A call is a job, `usage-and-quota.md` § Usage ledger).

use std::io::Cursor;

use mp4parse::TrackType;

/// What the header says of the video.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mp4Facts {
    /// How long its longest track plays, in seconds.
    pub seconds: f64,
    /// The picture's size in pixels, from the video track's header.
    pub width: u32,
    pub height: u32,
    /// Whether it has a sound track.
    pub sound: bool,
}

/// Why bytes are not a video this reads.
#[derive(Debug, thiserror::Error)]
pub enum NotMp4 {
    #[error("the file is not an MP4: {0}")]
    Unreadable(mp4parse::Error),
    #[error("the MP4 has no video track")]
    NoVideo,
}

/// How long `track` plays, in seconds, as a player shows it: its edit
/// list's length, else its header's, both in the movie's timescale, else
/// its media's in its own. The media's alone counts what an edit list
/// trims, such as the samples an AAC encoder primes a sound track with.
fn presented(context: &mp4parse::MediaContext, track: &mp4parse::Track) -> Option<f64> {
    let movie = context.timescale.map(|scale| scale.0).filter(|scale| *scale > 0);
    let edited = track.edited_duration.map(|duration| duration.0);
    let header = track.tkhd.as_ref().map(|header| header.duration).filter(|duration| *duration > 0);
    if let (Some(duration), Some(scale)) = (edited.or(header), movie) {
        return Some(duration as f64 / scale as f64);
    }
    let (duration, scale) = (track.duration?.0, track.timescale?.0);
    (scale > 0).then(|| duration as f64 / scale as f64)
}

/// The facts of the MP4 `bytes`: an ISO base media file whose header
/// parses and that has a video track.
pub fn read(bytes: &[u8]) -> Result<Mp4Facts, NotMp4> {
    let context = mp4parse::read_mp4(&mut Cursor::new(bytes)).map_err(NotMp4::Unreadable)?;
    let mut seconds: f64 = 0.0;
    let mut size = None;
    let mut sound = false;
    for track in context.tracks.iter() {
        seconds = seconds.max(presented(&context, track).unwrap_or(0.0));
        match track.track_type {
            TrackType::Video if size.is_none() => {
                // The track header holds the size as 16.16 fixed point.
                size = track.tkhd.as_ref().map(|header| (header.width >> 16, header.height >> 16));
            }
            TrackType::Audio => sound = true,
            _ => {}
        }
    }
    let (width, height) = size.ok_or(NotMp4::NoVideo)?;
    Ok(Mp4Facts { seconds, width, height, sound })
}
