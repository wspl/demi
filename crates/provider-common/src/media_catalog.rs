//! The catalog of image, video and audio models: what
//! each vendor publishes of its media models, which no vendor's listing and no
//! subscription tells. The release carries a copy of
//! `catalog/media/v1/models.json`, taken at build time, and a backend built
//! from a branch downloads that branch's file at start and once a day, with
//! its `ETag`; it uses the newer of the two by the file's `updated` field.
//! A download that fails, does not decode or is not newer is logged and
//! changes nothing. The catalog describes and does not limit: a capability the
//! vendor does not state is left out and means unknown, and a command warns
//! when the agent asks for what the catalog does not list but sends the request
//! anyway, since a vendor may take more than it wrote down.
//!
//! The types below are the file's shape, the one decoder every reader and
//! the repository's own check use: `vendor`, `id`, `kind` and `name` are
//! required, every other field is optional, and a field the backend does
//! not know is ignored, so a field added to the file keeps its version. A
//! change an older backend would misread, a field renamed or its meaning
//! changed, starts the next directory, `v2`; the old directory stays and is
//! still updated with new models for as long as a release that reads it is
//! supported.

use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};
use std::time::Duration;

use demi_shared_types::Timestamp;
use reqwest::Url;
use serde::Deserialize;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::conditional::{self, Answer, Failure, Validators};

/// The repository the catalog is downloaded from, and the file of the
/// schema version this backend reads, under a branch.
const REPOSITORY: &str = "https://raw.githubusercontent.com/wspl/demi";
const FILE: [&str; 4] = ["catalog", "media", "v1", "models.json"];

/// How often the file is downloaded again after the start.
const REFRESH_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long one download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// The file as the release carries it.
const CARRIED: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalog/media/v1/models.json"));

/// The address of the catalog of the build's `branch`; none for a build
/// from no branch, which keeps the copy it carries.
pub fn branch_url(branch: Option<&str>) -> Option<Url> {
    let branch = branch?;
    let mut url = Url::parse(REPOSITORY).expect("the repository's address parses");
    url.path_segments_mut()
        .expect("an https address has a path")
        .extend(branch.split('/'))
        .extend(FILE);
    Some(url)
}

/// The catalog: when it was last changed, the vendors it names that
/// models.dev does not list, and its models, in the file's order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Catalog {
    pub updated: Timestamp,
    /// The vendors that make media and no text, which models.dev, a
    /// catalog of text models, does not list (`crates/backend/src/families.rs`).
    #[serde(default)]
    pub vendors: Vec<CatalogVendor>,
    pub models: Vec<MediaModel>,
}

/// A vendor of media alone, as Add Provider offers it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CatalogVendor {
    /// Its id, which its models' `vendor` names.
    pub id: String,
    pub name: String,
    /// The API's address an entry of it starts with.
    pub endpoint: String,
    /// The vendor's API reference.
    pub doc: Option<String>,
    /// The vendor's pages the entry was checked against.
    #[serde(default)]
    pub sources: Vec<String>,
}

/// One model a vendor publishes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaModel {
    /// The vendor, such as `openai`, `google` or `xai`.
    pub vendor: String,
    /// The model's ID in the vendor's API.
    pub id: String,
    pub kind: MediaKind,
    /// The vendor's name for the model.
    pub name: String,
    /// The vendor's one-line description.
    pub description: Option<String>,
    /// The vendor's release date.
    pub released: Option<jiff::civil::Date>,
    /// For an alias, the ID it names now, whose release date, capabilities
    /// and price it has unless it states its own.
    pub alias_of: Option<String>,
    pub image: Option<ImageCapabilities>,
    pub video: Option<VideoCapabilities>,
    pub audio: Option<AudioCapabilities>,
    /// For a vendor whose models take different requests, which one the
    /// model takes, read only by that vendor's requests, such as MiniMax's
    /// `v2` for its H3 models and `v1` for Hailuo.
    pub api: Option<String>,
    pub price: Option<Price>,
    /// The subscriptions that make the model without listing it.
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
    /// The vendor's pages the entry was checked against.
    #[serde(default)]
    pub sources: Vec<String>,
}

/// What a model makes. A kind this backend does not know is read and
/// skipped by every reader, as an unknown field is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    #[serde(other)]
    Other,
}

impl From<demi_shared_types::MediaKind> for MediaKind {
    /// The catalog's kind of the models an entry makes of `kind`.
    fn from(kind: demi_shared_types::MediaKind) -> Self {
        match kind {
            demi_shared_types::MediaKind::Image => Self::Image,
            demi_shared_types::MediaKind::Video => Self::Video,
            demi_shared_types::MediaKind::Audio => Self::Audio,
        }
    }
}

/// A subscription that makes a model without listing it. One this
/// backend does not know is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Subscription {
    Chatgpt,
    Grok,
    #[serde(other)]
    Other,
}

/// What an image model takes, as the vendor states it; what it leaves out
/// is unknown.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCapabilities {
    /// The only sizes it takes, written `1024x1536`.
    pub sizes: Option<Vec<ImageSize>>,
    /// The rule of the sizes it takes besides, for a model that takes any
    /// size within it.
    pub custom_size: Option<CustomSize>,
    /// The shapes it takes, such as `16:9`.
    pub aspect_ratios: Option<Vec<String>>,
    /// The resolution tiers it takes, such as `2K`, as the vendor writes
    /// them.
    pub resolutions: Option<Vec<String>>,
    /// Whether it edits images.
    pub edits: Option<bool>,
    /// How many images an edit takes at most.
    pub max_input_images: Option<u32>,
    /// Whether it makes a transparent background.
    pub transparent: Option<bool>,
}

/// A size in pixels, which the file writes `1024x1536`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

impl TryFrom<String> for ImageSize {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let parsed = text
            .split_once('x')
            .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)));
        match parsed {
            Some((width, height)) if width > 0 && height > 0 => Ok(Self { width, height }),
            _ => Err(format!("{text:?} is no size of the form 1024x1536")),
        }
    }
}

/// The sizes a model takes besides any fixed ones.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSize {
    /// What each edge is a multiple of.
    pub multiple_of: Option<u32>,
    /// The longest an edge may be.
    pub max_edge: Option<u32>,
    /// How many times the shorter edge the longer may be.
    pub max_ratio: Option<u32>,
    pub min_pixels: Option<u64>,
    pub max_pixels: Option<u64>,
}

/// What a video model takes, as the vendor states it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCapabilities {
    /// The only durations it takes, in seconds.
    pub durations: Option<Vec<u32>>,
    /// The range of whole seconds it takes.
    pub duration_range: Option<DurationRange>,
    /// Such as `720p`.
    pub resolutions: Option<Vec<String>>,
    pub aspect_ratios: Option<Vec<String>>,
    /// Whether it takes the first frame, and the last.
    pub first_frame: Option<bool>,
    pub last_frame: Option<bool>,
    /// How many reference images it takes; 0 for none.
    pub max_reference_images: Option<u32>,
    /// Whether it makes sound.
    pub audio: Option<bool>,
}

/// The range of durations a model takes, in seconds, ends included; an
/// end the vendor does not state is left out.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct DurationRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// What an audio model takes and makes, as the vendor states it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCapabilities {
    /// What it makes: speech, music or sound effects, any of them.
    pub makes: Option<Vec<AudioOutput>>,
    /// For speech, its voices.
    pub voices: Option<Voices>,
    /// For speech, the languages it speaks, as the vendor names them.
    pub languages: Option<Vec<String>>,
    /// For speech, whether it takes instructions on the delivery, such as
    /// a tone or an accent.
    pub instructions: Option<bool>,
    /// For speech, the longest text one request takes, in characters, as
    /// the vendor states it, such as OpenAI's 4,096.
    pub max_characters: Option<u32>,
    /// For music and effects, the only durations it makes, in seconds.
    pub durations: Option<Vec<u32>>,
    /// For music and effects, the range of durations it makes.
    pub duration_range: Option<DurationRange>,
    /// For music, whether it takes lyrics.
    pub lyrics: Option<bool>,
    /// For music, whether it makes an instrumental on request.
    pub instrumental: Option<bool>,
    /// For effects and music, whether it makes a sound that loops.
    pub loops: Option<bool>,
    /// The formats it returns, such as `mp3` or `wav`.
    pub formats: Option<Vec<String>>,
}

/// What an audio model makes. One this backend does not know is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioOutput {
    Speech,
    Music,
    Effect,
    #[serde(other)]
    Other,
}

/// A speech model's voices: their names, or that the vendor lists them,
/// as one whose voices are the account's or many, written `"listed"`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Voices {
    Named(Vec<String>),
    Listed(ListedVoices),
}

/// The vendor lists the voices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListedVoices {
    Listed,
}

/// What a model costs: its rates, as the vendor states them, and the
/// vendor's pricing page.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Price {
    /// One rate per unit and per size, quality, resolution or duration
    /// where the price depends on them, such as a model's input and output
    /// tokens, each a rate of its own.
    pub rates: Vec<Rate>,
    /// The vendor's pricing page.
    pub page: Option<String>,
}

/// What one unit of a model's input or output costs, in the currency the
/// vendor charges in, never converted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Rate {
    pub unit: PriceUnit,
    /// The tokens' kind, where the vendor prices image and text tokens
    /// apart, as OpenAI does gpt-image-2's input.
    pub modality: Option<Modality>,
    pub amount: f64,
    pub currency: Currency,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub resolution: Option<String>,
    /// The length in seconds of the output the rate is for, such as a
    /// video of 6 seconds.
    pub duration: Option<f64>,
}

/// What a rate is the price of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PriceUnit {
    Image,
    /// One whole video, whatever its length within the rate's duration.
    Video,
    Second,
    Minute,
    Song,
    /// A million characters of text read aloud.
    MillionCharacters,
    MillionInputTokens,
    /// A million input tokens the vendor has cached from an earlier call.
    MillionCachedInputTokens,
    MillionOutputTokens,
    /// A unit this backend does not know, whose rate it cannot use.
    #[serde(other)]
    Other,
}

/// The kind of tokens a rate is the price of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Image,
    Text,
    /// A kind this backend does not know, whose rate it cannot use.
    #[serde(other)]
    Other,
}

/// The currency a vendor charges in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    Usd,
    /// Chinese yuan.
    Cny,
    /// A currency this backend does not know.
    #[serde(other)]
    Other,
}

impl Catalog {
    /// The catalog in `body`, or why it does not decode, naming the field.
    /// An alias names a model of its vendor that the file has and that is
    /// no alias itself; a file where one does not is refused whole.
    pub fn decode(body: &[u8]) -> Result<Self, String> {
        let mut json = serde_json::Deserializer::from_slice(body);
        let catalog: Self = serde_path_to_error::deserialize(&mut json)
            .map_err(|error| format!("it cannot be read at {}: {}", error.path(), error.inner()))?;
        json.end().map_err(|error| format!("it cannot be read: {error}"))?;
        for (index, model) in catalog.models.iter().enumerate() {
            let Some(named) = &model.alias_of else {
                continue;
            };
            match catalog.model(&model.vendor, named) {
                Some(target) if target.alias_of.is_none() => {}
                Some(_) => {
                    return Err(format!(
                        "it cannot be read at models[{index}].aliasOf: {named} is an alias, not a model"
                    ));
                }
                None => {
                    return Err(format!(
                        "it cannot be read at models[{index}].aliasOf: {} has no model {named}",
                        model.vendor
                    ));
                }
            }
        }
        Ok(catalog)
    }

    /// The copy the release carries.
    pub fn carried() -> Arc<Self> {
        static CARRIED_CATALOG: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
            // The repository's checks decode this file, so a release
            // carries one that decodes.
            Arc::new(Catalog::decode(CARRIED.as_bytes()).expect("the carried media catalog decodes"))
        });
        CARRIED_CATALOG.clone()
    }

    /// `vendor`'s model `id`.
    pub fn model(&self, vendor: &str, id: &str) -> Option<&MediaModel> {
        self.models.iter().find(|model| model.vendor == vendor && model.id == id)
    }

    /// The model `model` names, when it is an alias the catalog has.
    fn named_by<'a>(&'a self, model: &MediaModel) -> Option<&'a MediaModel> {
        let target = model.alias_of.as_deref()?;
        self.model(&model.vendor, target)
    }

    /// `model`'s release date: its own, else that of the model it names.
    pub fn released(&self, model: &MediaModel) -> Option<jiff::civil::Date> {
        model.released.or_else(|| self.named_by(model).and_then(|named| named.released))
    }

    /// What image `model` takes: its own, else that of the model it names.
    pub fn image<'a>(&'a self, model: &'a MediaModel) -> Option<&'a ImageCapabilities> {
        model.image.as_ref().or_else(|| self.named_by(model).and_then(|named| named.image.as_ref()))
    }

    /// What video `model` takes: its own, else that of the model it names.
    pub fn video<'a>(&'a self, model: &'a MediaModel) -> Option<&'a VideoCapabilities> {
        model.video.as_ref().or_else(|| self.named_by(model).and_then(|named| named.video.as_ref()))
    }

    /// What audio `model` makes: its own, else that of the model it names.
    pub fn audio<'a>(&'a self, model: &'a MediaModel) -> Option<&'a AudioCapabilities> {
        model.audio.as_ref().or_else(|| self.named_by(model).and_then(|named| named.audio.as_ref()))
    }

    /// What `model` costs: its own, else that of the model it names.
    pub fn price<'a>(&'a self, model: &'a MediaModel) -> Option<&'a Price> {
        model.price.as_ref().or_else(|| self.named_by(model).and_then(|named| named.price.as_ref()))
    }
}

/// The catalog a backend uses, the newer of its release's copy and the
/// last one downloaded. Cloning it is cheap, and every clone shares the
/// catalog and the daily download.
#[derive(Clone)]
pub struct MediaCatalog {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    /// The file of the build's branch; none for a build from no branch.
    url: Option<Url>,
    /// The catalog in use, which announces each change to its readers.
    catalog: watch::Sender<Arc<Catalog>>,
    state: Mutex<State>,
}

struct State {
    /// What revalidates the last file downloaded, used or not, so an
    /// unchanged file is not downloaded again.
    validators: Option<Validators>,
    /// The daily download, while it runs.
    refreshes: Option<AbortHandle>,
}

impl MediaCatalog {
    /// The catalog of `carried`, the release's copy ([`Catalog::carried`])
    /// or a test's, which `start` keeps current from `url`, the file of the
    /// build's branch ([`branch_url`]).
    pub fn new(http: reqwest::Client, url: Option<Url>, carried: Arc<Catalog>) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                url,
                catalog: watch::Sender::new(carried),
                state: Mutex::new(State {
                    validators: None,
                    refreshes: None,
                }),
            }),
        }
    }

    /// The catalog now.
    pub fn current(&self) -> Arc<Catalog> {
        self.inner.catalog.borrow().clone()
    }

    /// The catalog now, and each catalog that replaces it.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Catalog>> {
        self.inner.catalog.subscribe()
    }

    /// Downloads the file now and then once a day on `edge`, until
    /// [`close`](Self::close) or the last clone is dropped; a build from no
    /// branch downloads nothing. A failed download waits for the next day.
    pub fn start(&self, edge: &Handle) {
        if self.inner.url.is_none() {
            return;
        }
        // The task holds the catalog weakly, so it ends with the catalog's
        // owners even if nobody closes it.
        let catalog: Weak<Inner> = Arc::downgrade(&self.inner);
        let task = edge.spawn(async move {
            let mut every = tokio::time::interval(REFRESH_EVERY);
            loop {
                every.tick().await;
                let Some(inner) = catalog.upgrade() else {
                    return;
                };
                MediaCatalog { inner }.refresh().await;
            }
        });
        let replaced = self.inner.lock().refreshes.replace(task.abort_handle());
        if let Some(replaced) = replaced {
            replaced.abort();
        }
    }

    /// Stops the daily download, a download under way included.
    pub fn close(&self) {
        if let Some(refreshes) = self.inner.lock().refreshes.take() {
            refreshes.abort();
        }
    }

    /// Downloads the file once and uses it when it is newer than the
    /// catalog now; a failure, or a file that does not decode or is not
    /// newer, is logged and changes nothing.
    pub async fn refresh(&self) {
        let Some(url) = &self.inner.url else {
            return;
        };
        let validators = self.inner.lock().validators.clone();
        let answer = tokio::time::timeout(
            DOWNLOAD_TIMEOUT,
            conditional::get(&self.inner.http, url, validators.as_ref()),
        )
        .await;
        let (body, validators) = match answer {
            Ok(Ok(Answer::NotModified)) => return,
            Ok(Ok(Answer::Fetched { body, validators })) => (body, validators),
            Ok(Err(Failure::Status(status))) => {
                tracing::warn!("the media catalog at {url} was not downloaded: HTTP {}", status.as_u16());
                return;
            }
            Ok(Err(Failure::Transport(error))) => {
                tracing::warn!("the media catalog at {url} was not downloaded: {error}");
                return;
            }
            Err(_) => {
                tracing::warn!("the media catalog at {url} was not downloaded within {DOWNLOAD_TIMEOUT:?}");
                return;
            }
        };
        let decoded = Catalog::decode(&body);
        self.inner.lock().validators = Some(validators);
        let downloaded = match decoded {
            Ok(downloaded) => downloaded,
            Err(error) => {
                tracing::warn!("the media catalog at {url} is not used: {error}");
                return;
            }
        };
        self.inner.catalog.send_if_modified(|catalog| {
            if downloaded.updated > catalog.updated {
                *catalog = Arc::new(downloaded);
                return true;
            }
            // An equal date is the file in use.
            if downloaded.updated < catalog.updated {
                tracing::warn!(
                    "the media catalog at {url} is not used: it was updated at {}, before the one in use, of {}",
                    downloaded.updated,
                    catalog.updated
                );
            }
            false
        });
    }
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A section under the lock only reads or replaces fields, so a
        // panic elsewhere leaves the state whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
