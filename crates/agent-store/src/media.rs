//! Media by reference (`runtime.md` § Media): a medium is stored once, as a
//! blob in the conversation owner's namespace, and blocks hold its reference
//! everywhere. A session holds the bytes of the media its replayed blocks
//! and its waiting input reference, and the model's view is the replayed
//! blocks with what the session holds for their media, from which replay
//! puts the bytes into a request. Where the bytes go is the store's
//! decision, and the agent's server never holds a blob namespace.

use std::collections::{HashMap, HashSet};

use demi_shared_types::{
    B64Bytes, BlobRef, Block, DocumentSource, GoneCause, MediaSource, ModelMediaKind,
    ToolCallBlock, ToolMediaSource, ToolResultContentBlock, ToolView, UserContentBlock,
};
use demi_provider_common::{MediaBytes, ResultPart};
use futures_util::{StreamExt as _, TryStreamExt as _, future::LocalBoxFuture, stream};

use super::StoreError;

/// How many blobs a session reads at once: on S3 each read is a round trip.
const READS_AT_ONCE: usize = 8;

/// The conversation owner's blob namespace, as a session reaches it through
/// its store.
pub trait BlobStore {
    /// Stores `bytes` under the SHA-256 that names them, unless the namespace
    /// holds that blob already, and answers the name.
    fn put(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, StoreError>>;

    /// The bytes of `blob`; none when the namespace does not hold it.
    fn get<'a>(
        &'a self,
        blob: &'a BlobRef,
    ) -> LocalBoxFuture<'a, Result<Option<B64Bytes>, StoreError>>;
}

/// What a session holds for one medium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    Bytes(B64Bytes),
    /// The namespace does not hold the medium's blob.
    Missing,
}

/// The media a session holds, each by the blob that names it: its bytes, or
/// the fact that its blob is missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeldMedia(HashMap<BlobRef, Held>);

impl HeldMedia {
    /// Holds `bytes`, which `blob` names, unless something is held for it
    /// already: a medium reaches the model in one form.
    pub fn hold(&mut self, blob: BlobRef, bytes: B64Bytes) {
        self.0.entry(blob).or_insert(Held::Bytes(bytes));
    }

    /// Holds what `other` holds, keeping what is held already.
    pub fn absorb(&mut self, other: Self) {
        for (blob, held) in other.0 {
            self.0.entry(blob).or_insert(held);
        }
    }

    /// Lets go of every medium `referenced` does not name.
    pub fn retain(&mut self, referenced: &HashSet<BlobRef>) {
        self.0.retain(|blob, _| referenced.contains(blob));
    }

    /// What is held for `blob`; none when nothing is.
    pub(crate) fn get(&self, blob: &BlobRef) -> Option<&Held> {
        self.0.get(blob)
    }

    /// The blobs `blocks` reference that nothing is held for, each once.
    fn unheld(&self, blocks: &[Block]) -> Vec<BlobRef> {
        let mut unheld = Vec::new();
        for blob in blocks.iter().flat_map(references) {
            if !self.0.contains_key(blob) && !unheld.contains(blob) {
                unheld.push(blob.clone());
            }
        }
        unheld
    }

    /// What is held for the media `blocks` reference.
    pub fn select(&self, blocks: &[Block]) -> Self {
        let selected = blocks
            .iter()
            .flat_map(references)
            .filter_map(|blob| Some((blob.clone(), self.0.get(blob)?.clone())))
            .collect();
        Self(selected)
    }
}

/// The replayed blocks as the model receives them (`runtime.md` § Media):
/// the blocks as the transcript holds them, by reference, with what the
/// session holds for each medium they reference. Replay, the token
/// estimates and compaction read it.
pub struct ModelView {
    /// Where the blocks start in the transcript.
    pub start: usize,
    pub blocks: Vec<Block>,
    pub(crate) media: HeldMedia,
}

impl ModelView {
    /// The view of `blocks`, which start at `start` in the transcript, with
    /// what `held` holds for their media. When nothing is held for some of
    /// them, there is no view yet: the blobs to read first.
    pub fn of(
        start: usize,
        blocks: &[Block],
        held: &HeldMedia,
    ) -> Result<Self, Vec<BlobRef>> {
        let unheld = held.unheld(blocks);
        if !unheld.is_empty() {
            return Err(unheld);
        }
        Ok(Self {
            start,
            blocks: blocks.to_vec(),
            media: held.select(blocks),
        })
    }

    /// What the session holds for `blob`, which one of the view's blocks
    /// references.
    pub fn held(&self, blob: &BlobRef) -> &Held {
        self.media
            .get(blob)
            .expect("the model's view holds something for every medium its blocks reference")
    }
}

/// The text a medium whose blob is missing becomes in a request, and in
/// the estimate: `[missing <kind>]`, where the kind is `image`, `video` or
/// `document`; the model has no use for the blob's name.
pub fn missing_text(kind: &str) -> String {
    format!("[missing {kind}]")
}

/// Where one medium of a block is.
#[derive(Clone, Copy)]
enum Source<'a> {
    /// A message's image or video.
    Media(&'a MediaSource),
    /// A message's document.
    Document(&'a DocumentSource),
    /// A tool result's image or video.
    Tool(&'a ToolMediaSource),
}

impl<'a> Source<'a> {
    /// The blob the medium references; none for a URL.
    fn reference(self) -> Option<&'a BlobRef> {
        match self {
            Self::Media(MediaSource::Ref { r#ref, .. })
            | Self::Document(DocumentSource::Ref { r#ref, .. })
            | Self::Tool(ToolMediaSource::Ref { r#ref, .. }) => Some(r#ref),
            Self::Media(MediaSource::Url { .. }) => None,
        }
    }
}

/// The media of a message's or a steer's content.
fn content_sources(content: &[UserContentBlock]) -> impl Iterator<Item = Source<'_>> {
    content.iter().filter_map(|part| match part {
        UserContentBlock::Image { source } | UserContentBlock::Video { source } => {
            Some(Source::Media(source))
        }
        UserContentBlock::Document { source } => Some(Source::Document(source)),
        _ => None,
    })
}

/// The media of a block: a message's or a steer's, or a tool result's.
fn sources(block: &Block) -> Vec<Source<'_>> {
    match block {
        Block::User(user) => content_sources(&user.content).collect(),
        Block::Steer(steer) => content_sources(&steer.content).collect(),
        Block::ToolCall(call) => call
            .output
            .iter()
            .filter_map(|part| match part {
                ToolResultContentBlock::Image { source }
                | ToolResultContentBlock::Video { source } => Some(Source::Tool(source)),
                ToolResultContentBlock::Text { .. } | ToolResultContentBlock::Gone { .. } => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The blobs a block's media reference.
pub fn references(block: &Block) -> impl Iterator<Item = &BlobRef> {
    sources(block).into_iter().filter_map(Source::reference)
}

/// What in a block holds a blob it references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Holder {
    /// A message's or a steer's medium, from the user's uploads.
    Message,
    /// A tool result's image or video, which retirement may take
    /// (`runtime.md` § Retired tool media).
    ToolResult,
    /// A side of a file a shell call's command edited, which only the user
    /// sees (`edit-tracking.md` § Edit copies).
    EditCopy,
}

/// One blob a block references, as a store indexes it for retention
/// (`storage.md` § Retention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockReference<'a> {
    pub blob: &'a BlobRef,
    pub holder: Holder,
}

/// The blobs a block references: its media in the order its parts hold
/// them, then its edit copies, file by file and segment by segment, the
/// original before the modified side.
pub fn block_references(block: &Block) -> Vec<BlockReference<'_>> {
    let media = sources(block).into_iter().filter_map(|source| {
        Some(BlockReference {
            blob: source.reference()?,
            holder: match source {
                Source::Tool(_) => Holder::ToolResult,
                Source::Media(_) | Source::Document(_) => Holder::Message,
            },
        })
    });
    let files = match block {
        Block::ToolCall(ToolCallBlock {
            view: Some(ToolView::Shell(view)),
            ..
        }) => view.files.as_deref().unwrap_or_default(),
        _ => &[],
    };
    let copies = files
        .iter()
        .flat_map(|file| &file.edits)
        .filter_map(|edit| edit.copies.as_ref())
        .flat_map(|copies| [&copies.original, &copies.modified])
        .map(|blob| BlockReference {
            blob,
            holder: Holder::EditCopy,
        });
    media.chain(copies).collect()
}

/// The blobs the media of a message's or a steer's content reference, such
/// as a queued message's.
pub fn content_references(content: &[UserContentBlock]) -> impl Iterator<Item = &BlobRef> {
    content_sources(content).filter_map(Source::reference)
}

/// Stores the media of a tool's result as the result enters the transcript
/// (`runtime.md` § Media): each medium's bytes are put, the result keeps
/// the reference and the bytes are held. A medium whose put fails is gone
/// from the result, not stored, with the store's error, so no block names a
/// blob that was not stored.
pub async fn store_result(
    output: Vec<ResultPart>,
    blobs: &dyn BlobStore,
) -> (Vec<ToolResultContentBlock>, HeldMedia) {
    let mut held = HeldMedia::default();
    let mut stored = Vec::with_capacity(output.len());
    for part in output {
        let (kind, MediaBytes { data, media_type }) = match part {
            ResultPart::Text(text) => {
                stored.push(ToolResultContentBlock::Text { text });
                continue;
            }
            ResultPart::Image(bytes) => (ModelMediaKind::Image, bytes),
            ResultPart::Video(bytes) => (ModelMediaKind::Video, bytes),
        };
        let part = match blobs.put(data.clone()).await {
            Ok(blob) => {
                held.hold(blob.clone(), data);
                let source = ToolMediaSource::Ref {
                    r#ref: blob,
                    media_type,
                };
                match kind {
                    ModelMediaKind::Image => ToolResultContentBlock::Image { source },
                    ModelMediaKind::Video => ToolResultContentBlock::Video { source },
                }
            }
            Err(error) => ToolResultContentBlock::Gone {
                kind,
                media_type,
                cause: GoneCause::NotStored {
                    error: error.to_string(),
                },
            },
        };
        stored.push(part);
    }
    (stored, held)
}

/// Reads `blobs` from `store`, a few at a time: each one's bytes, or that it
/// is missing.
pub async fn read(store: &dyn BlobStore, blobs: Vec<BlobRef>) -> Result<HeldMedia, StoreError> {
    let found: Vec<(BlobRef, Option<B64Bytes>)> = stream::iter(blobs)
        .map(|blob| async move {
            let bytes = store.get(&blob).await?;
            Ok::<_, StoreError>((blob, bytes))
        })
        .buffer_unordered(READS_AT_ONCE)
        .try_collect()
        .await?;
    let held = found
        .into_iter()
        .map(|(blob, bytes)| (blob, bytes.map_or(Held::Missing, Held::Bytes)))
        .collect();
    Ok(HeldMedia(held))
}
