//! Media by reference (`runtime.md` § Media): a medium is stored once, as a
//! blob in the conversation owner's namespace, and blocks hold its reference
//! everywhere. A session holds the bytes of the media its replayed blocks
//! and its waiting input reference, and the model's view puts them in the
//! references' place. This module is the one mapping between the two forms;
//! where the bytes go is the store's decision, and the agent's server never
//! holds a blob namespace.

use std::collections::{HashMap, HashSet};

use demi_core::{
    B64Bytes, BlobRef, Block, DocumentSource, MediaSource, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock,
};
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
enum Held {
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
    pub(crate) fn retain(&mut self, referenced: &HashSet<BlobRef>) {
        self.0.retain(|blob, _| referenced.contains(blob));
    }

    /// The blobs `blocks` reference that nothing is held for, each once.
    pub(crate) fn unheld(&self, blocks: &[Block]) -> Vec<BlobRef> {
        let mut unheld = Vec::new();
        for blob in blocks.iter().flat_map(references) {
            if !self.0.contains_key(blob) && !unheld.contains(blob) {
                unheld.push(blob.clone());
            }
        }
        unheld
    }

    /// What is held for the media `blocks` reference.
    pub(crate) fn select(&self, blocks: &[Block]) -> Self {
        let selected = blocks
            .iter()
            .flat_map(references)
            .filter_map(|blob| Some((blob.clone(), self.0.get(blob)?.clone())))
            .collect();
        Self(selected)
    }

    /// The model's view of `blocks`: each held medium's bytes in the place of
    /// its reference, and the text `[missing <kind> blob <ref>]` in the place
    /// of a medium whose blob is missing. A medium nothing is held for keeps
    /// its reference.
    pub(crate) fn view(&self, blocks: &[Block]) -> Vec<Block> {
        blocks
            .iter()
            .map(|block| {
                let mut viewed = block.clone();
                match &mut viewed {
                    Block::User(user) => self.view_content(&mut user.content),
                    Block::Steer(steer) => self.view_content(&mut steer.content),
                    Block::ToolCall(call) => {
                        for part in &mut call.output {
                            if let Some(viewed) = self.view_result_part(part) {
                                *part = viewed;
                            }
                        }
                    }
                    _ => {}
                }
                viewed
            })
            .collect()
    }

    fn view_content(&self, content: &mut [UserContentBlock]) {
        for part in content {
            if let Some(viewed) = self.view_content_part(part) {
                *part = viewed;
            }
        }
    }

    /// What a message's part becomes in the model's view; none when it
    /// stays as it is.
    fn view_content_part(&self, part: &UserContentBlock) -> Option<UserContentBlock> {
        let viewed = match part {
            UserContentBlock::Image {
                source: MediaSource::Ref { r#ref, media_type },
            } => match self.0.get(r#ref)? {
                Held::Bytes(data) => UserContentBlock::Image {
                    source: MediaSource::Binary {
                        data: data.clone(),
                        media_type: media_type.clone(),
                    },
                },
                Held::Missing => missing_text("image", r#ref),
            },
            UserContentBlock::Video {
                source: MediaSource::Ref { r#ref, media_type },
            } => match self.0.get(r#ref)? {
                Held::Bytes(data) => UserContentBlock::Video {
                    source: MediaSource::Binary {
                        data: data.clone(),
                        media_type: media_type.clone(),
                    },
                },
                Held::Missing => missing_text("video", r#ref),
            },
            UserContentBlock::Document {
                source:
                    DocumentSource::Ref {
                        r#ref,
                        media_type,
                        file_name,
                    },
            } => match self.0.get(r#ref)? {
                Held::Bytes(data) => UserContentBlock::Document {
                    source: DocumentSource::Binary {
                        data: data.clone(),
                        media_type: media_type.clone(),
                        file_name: file_name.clone(),
                    },
                },
                Held::Missing => missing_text("document", r#ref),
            },
            _ => return None,
        };
        Some(viewed)
    }

    /// What a tool result's part becomes in the model's view; none when it
    /// stays as it is.
    fn view_result_part(&self, part: &ToolResultContentBlock) -> Option<ToolResultContentBlock> {
        let (kind, source) = match part {
            ToolResultContentBlock::Image { source } => (ToolMedium::Image, source),
            ToolResultContentBlock::Video { source } => (ToolMedium::Video, source),
            ToolResultContentBlock::Text { .. } => return None,
        };
        let ToolMediaSource::Ref { r#ref, media_type } = source else {
            return None;
        };
        let viewed = match self.0.get(r#ref)? {
            Held::Bytes(data) => kind.part(ToolMediaSource::Binary {
                data: data.clone(),
                media_type: media_type.clone(),
            }),
            Held::Missing => ToolResultContentBlock::Text {
                text: missing(kind.name(), r#ref),
            },
        };
        Some(viewed)
    }
}

/// A tool result's kind of medium.
#[derive(Debug, Clone, Copy)]
enum ToolMedium {
    Image,
    Video,
}

impl ToolMedium {
    fn name(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
        }
    }

    /// The result part that holds `source` as this kind of medium.
    fn part(self, source: ToolMediaSource) -> ToolResultContentBlock {
        match self {
            Self::Image => ToolResultContentBlock::Image { source },
            Self::Video => ToolResultContentBlock::Video { source },
        }
    }
}

fn missing(kind: &str, blob: &BlobRef) -> String {
    format!("[missing {kind} blob {blob}]")
}

fn missing_text(kind: &str, blob: &BlobRef) -> UserContentBlock {
    UserContentBlock::Text {
        text: missing(kind, blob),
    }
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
    /// The blob the medium references; none when it holds its bytes or a
    /// URL.
    fn reference(self) -> Option<&'a BlobRef> {
        match self {
            Self::Media(MediaSource::Ref { r#ref, .. })
            | Self::Document(DocumentSource::Ref { r#ref, .. })
            | Self::Tool(ToolMediaSource::Ref { r#ref, .. }) => Some(r#ref),
            _ => None,
        }
    }

    fn has_bytes(self) -> bool {
        matches!(
            self,
            Self::Media(MediaSource::Binary { .. })
                | Self::Document(DocumentSource::Binary { .. })
                | Self::Tool(ToolMediaSource::Binary { .. })
        )
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
                ToolResultContentBlock::Text { .. } => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The blobs a block's media reference.
pub(crate) fn references(block: &Block) -> impl Iterator<Item = &BlobRef> {
    sources(block).into_iter().filter_map(Source::reference)
}

/// One medium a block references, as a store indexes it for retention
/// (`storage.md` § Retention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockMedium<'a> {
    pub blob: &'a BlobRef,
    /// Whether a tool result holds it, which retirement may take
    /// (`runtime.md` § Retired tool media); a message's and a steer's come
    /// from the user's uploads.
    pub tool: bool,
}

/// The media a block references, in the order its parts hold them.
pub fn block_media(block: &Block) -> Vec<BlockMedium<'_>> {
    sources(block)
        .into_iter()
        .filter_map(|source| {
            Some(BlockMedium {
                blob: source.reference()?,
                tool: matches!(source, Source::Tool(_)),
            })
        })
        .collect()
}

/// The blobs the media of a message's or a steer's content reference, such
/// as a queued message's.
pub fn content_references(content: &[UserContentBlock]) -> impl Iterator<Item = &BlobRef> {
    content_sources(content).filter_map(Source::reference)
}

/// Whether one of a block's media holds its bytes instead of a reference.
pub(crate) fn holds_bytes(block: &Block) -> bool {
    sources(block).into_iter().any(Source::has_bytes)
}

/// Whether one of the media of a message's content holds its bytes instead
/// of a reference.
pub(crate) fn content_holds_bytes(content: &[UserContentBlock]) -> bool {
    content_sources(content).any(Source::has_bytes)
}

/// Stores the media of a tool's result as the result enters the transcript
/// (`runtime.md` § Media): each medium's bytes are put, the result keeps
/// the reference and the bytes are held. A medium whose put fails becomes
/// the text `[<kind> not stored: <reason>]`, so no block names a blob that
/// was not stored.
pub(crate) async fn store_result(
    output: Vec<ToolResultContentBlock>,
    blobs: &dyn BlobStore,
) -> (Vec<ToolResultContentBlock>, HeldMedia) {
    let mut held = HeldMedia::default();
    let mut stored = Vec::with_capacity(output.len());
    for part in output {
        let (kind, data, media_type) = match part {
            ToolResultContentBlock::Image {
                source: ToolMediaSource::Binary { data, media_type },
            } => (ToolMedium::Image, data, media_type),
            ToolResultContentBlock::Video {
                source: ToolMediaSource::Binary { data, media_type },
            } => (ToolMedium::Video, data, media_type),
            other => {
                stored.push(other);
                continue;
            }
        };
        match blobs.put(data.clone()).await {
            Ok(blob) => {
                stored.push(kind.part(ToolMediaSource::Ref {
                    r#ref: blob.clone(),
                    media_type,
                }));
                held.hold(blob, data);
            }
            Err(error) => stored.push(ToolResultContentBlock::Text {
                text: format!("[{} not stored: {error}]", kind.name()),
            }),
        }
    }
    (stored, held)
}

/// Reads `blobs` from `store`, a few at a time: each one's bytes, or that it
/// is missing.
pub(crate) async fn read(store: &dyn BlobStore, blobs: Vec<BlobRef>) -> Result<HeldMedia, StoreError> {
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
