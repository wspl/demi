//! The video role of a provider (`videos.md` § Vendors): a provider that
//! makes videos lists its video models and runs one job for its account or
//! key in three steps, the ask, the reads of the job's state and the
//! download; the backend chooses the entry and the account, paces the
//! reads and bounds the job. What every vendor's request has in common is
//! in [`crate::media`]; the xAI Videos shape, which a Grok subscription, an
//! xAI key and a compatible API share, is in [`xai`].

pub mod xai;

use bytes::Bytes;
use demi_shared_types::{Clock, Cost, TokenUsage, VideoShape};
use futures_util::future::BoxFuture;
use http::HeaderMap;
use http::header::LOCATION;
use reqwest::Url;

use crate::images::{Aspect, InputImage};
use crate::media::{MediaAnswer, MediaCall, MediaCredential, MediaFailure, MediaMaker, unreadable};
use crate::read_http_failure;

/// A provider's video role.
pub trait VideoMaker: MediaMaker {
    /// The shape its requests take, which says the options it sends
    /// ([`unsent`]).
    fn shape(&self) -> VideoShape;

    /// Asks for the video `request` describes and answers the job's id.
    fn ask<'a>(&'a self, http: &'a crate::ProviderHttp, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>>;

    /// Reads the state of the job `job` once.
    fn state<'a>(&'a self, http: &'a crate::ProviderHttp, job: &'a str) -> BoxFuture<'a, MediaAnswer<JobState>>;

    /// Downloads the video of a finished job, with the entry's key where
    /// the vendor needs it.
    fn download<'a>(&'a self, http: &'a crate::ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>>;

    /// Cancels the job `job`, for a vendor that can cancel one; none
    /// elsewhere, where the job runs to its end and is paid for.
    fn cancel<'a>(&'a self, _http: &'a crate::ProviderHttp, _job: &'a str) -> Option<BoxFuture<'a, MediaAnswer<()>>> {
        None
    }
}

/// One video to make; an option left out is the vendor's default.
#[derive(Debug, Clone, Default)]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    /// The first frame.
    pub image: Option<InputImage>,
    /// The last frame, with a first.
    pub last_frame: Option<InputImage>,
    /// Images the video keeps to, such as a character or a product.
    pub references: Vec<InputImage>,
    /// Seconds.
    pub duration: Option<u32>,
    pub aspect: Option<Aspect>,
    pub resolution: Option<Resolution>,
    /// Whether the model makes sound.
    pub audio: Option<bool>,
    pub seed: Option<u64>,
}

/// The resolutions `--resolution` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    P480,
    P720,
    P1080,
    K4,
}

impl Resolution {
    /// As the vendors and `--resolution` write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P480 => "480p",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::K4 => "4k",
        }
    }

    /// The shorter side of a picture of this resolution, in pixels.
    pub fn short_side(self) -> u32 {
        match self {
            Self::P480 => 480,
            Self::P720 => 720,
            Self::P1080 => 1080,
            Self::K4 => 2160,
        }
    }
}

/// An option of `demi video generate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoOption {
    Image,
    LastFrame,
    Reference,
    Duration,
    Aspect,
    Resolution,
    Audio,
    Seed,
}

/// The options `request` gives that `shape` has no field for, which are
/// not sent (`videos.md` § Vendors, the option table), in the order of
/// the table.
pub fn unsent(shape: VideoShape, request: &VideoRequest) -> Vec<VideoOption> {
    let lacks: &[VideoOption] = match shape {
        VideoShape::Grok | VideoShape::XaiVideos => &[VideoOption::Seed],
        VideoShape::Veo => &[VideoOption::Audio],
        VideoShape::OpenaiVideos => &[VideoOption::LastFrame, VideoOption::Reference, VideoOption::Audio, VideoOption::Seed],
        VideoShape::OpenrouterVideos => &[],
    };
    let given = [
        (VideoOption::Image, request.image.is_some()),
        (VideoOption::LastFrame, request.last_frame.is_some()),
        (VideoOption::Reference, !request.references.is_empty()),
        (VideoOption::Duration, request.duration.is_some()),
        (VideoOption::Aspect, request.aspect.is_some()),
        (VideoOption::Resolution, request.resolution.is_some()),
        (VideoOption::Audio, request.audio.is_some()),
        (VideoOption::Seed, request.seed.is_some()),
    ];
    given
        .into_iter()
        .filter(|(option, given)| *given && lacks.contains(option))
        .map(|(option, _)| option)
        .collect()
}

/// A job's state as one read of it says.
#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    /// Waiting for the vendor to start it.
    Queued,
    /// Being made, with how far along when the vendor says.
    Running { progress: Option<u8> },
    /// Made, and where the video is.
    Done(Done),
    /// The vendor failed the video, in its own words.
    Failed { message: String },
    /// The job expired at the vendor before it ran.
    Expired,
}

/// A finished job: where its video is and what the vendor said it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub url: Url,
    /// The cost the vendor reported, in its documented unit; none
    /// otherwise.
    pub cost: Option<Cost>,
    /// The counts the vendor reported; none when it reported none.
    pub usage: Option<TokenUsage>,
}

/// The largest video a download takes, read as it arrives: a vendor's
/// longest, 15 seconds at 1080p, is tens of MiB (`videos.md` § The job).
pub const DOWNLOAD_LIMIT: usize = 256 * 1024 * 1024;

/// How many redirects a download follows, as reqwest's own policy does.
const REDIRECTS: usize = 10;

/// Downloads `url`, within the request deadline. The entry's `key`
/// headers go only to an address of the origin of the entry's `endpoint`,
/// never to another host's, a storage bucket's or a redirect's to another
/// origin, which each get the request without them; so redirects are
/// followed here, each hop's headers chosen by its origin. A video over
/// [`DOWNLOAD_LIMIT`] fails. A refusal is told in the vendor's words
/// under `label`; an address is no account's, so nothing is renewed and
/// no limit is read from it.
pub async fn download(
    label: &'static str,
    clock: &dyn Clock,
    http: &crate::ProviderHttp,
    url: &Url,
    endpoint: &Url,
    key: HeaderMap,
) -> MediaAnswer<Bytes> {
    let call = MediaCall {
        label,
        credential: MediaCredential::Key { refused: |_, _| false },
        reader: read_http_failure,
        used_up: |failure| failure,
        clock,
        quota: None,
    };
    let result = crate::media::within_deadline(async {
        let http = http.without_redirects().map_err(crate::media::transport)?;
        let origin = endpoint.origin();
        let mut url = url.clone();
        for _ in 0..=REDIRECTS {
            let headers = if url.origin() == origin { key.clone() } else { HeaderMap::new() };
            let mut response = http.get(url.clone()).headers(headers).send().await.map_err(crate::media::transport)?;
            let status = response.status();
            if status.is_redirection() {
                let location = response.headers().get(LOCATION).and_then(|location| location.to_str().ok());
                if let Some(next) = location.and_then(|location| url.join(location).ok()) {
                    url = next;
                    continue;
                }
            }
            if !status.is_success() {
                return Err(call.refusal(response).await);
            }
            if response.content_length().is_some_and(|length| length > DOWNLOAD_LIMIT as u64) {
                return Err(too_large());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(crate::media::transport)? {
                if body.len() + chunk.len() > DOWNLOAD_LIMIT {
                    return Err(too_large());
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(Bytes::from(body));
        }
        Err(unreadable(format!("its video's address redirected more than {REDIRECTS} times")))
    })
    .await;
    MediaAnswer { result, observed: Vec::new() }
}

fn too_large() -> MediaFailure {
    unreadable(format!("its video is over {} MiB", DOWNLOAD_LIMIT / (1024 * 1024)))
}
