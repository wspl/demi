//! Media by reference (`runtime.md` § Media): a session holds its media's
//! bytes for inference, while a store keeps them in the conversation
//! owner's blob namespace and saves references in their place, and the page
//! receives the references. This module is the one mapping between the two
//! forms; where the bytes go is the store's decision, and the agent server
//! never holds a blob namespace.

use demi_agent_protocol::{ServerFrame, TranscriptPatch};
use demi_core::{
    B64Bytes, BlobRef, Block, DocumentSource, MediaSource, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock,
};
use futures_util::future::LocalBoxFuture;

use super::{Checkpoint, CheckpointUpdate, StoreError};

/// The conversation owner's blob namespace, as a store or the conversation
/// socket reaches it.
pub trait BlobStore {
    /// Stores `bytes` under the SHA-256 that names them; the same bytes
    /// always name the same blob.
    fn put(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, StoreError>>;

    /// The bytes of `blob`; none when the namespace does not hold it.
    fn get<'a>(
        &'a self,
        blob: &'a BlobRef,
    ) -> LocalBoxFuture<'a, Result<Option<B64Bytes>, StoreError>>;
}

/// Moves the inline media of the blocks `update` saves into `blobs`, which a
/// store does before the save that references them.
pub async fn externalize_update(
    update: &mut CheckpointUpdate,
    blobs: &dyn BlobStore,
) -> Result<(), StoreError> {
    for (_, block) in &mut update.changed_blocks {
        externalize(block, blobs).await?;
    }
    Ok(())
}

/// Puts the bytes of the media references in `checkpoint`'s transcript back
/// from `blobs`, as a store loads a session for inference ([`rehydrate`]).
pub async fn rehydrate_checkpoint(
    checkpoint: &mut Checkpoint,
    blobs: &dyn BlobStore,
) -> Result<(), StoreError> {
    for block in &mut checkpoint.transcript {
        rehydrate(block, blobs).await?;
    }
    Ok(())
}

/// Moves `block`'s inline media into `blobs`: the images, videos and
/// documents of a message or a steer, and the images and videos of a tool's
/// result, keep references instead of their bytes.
pub async fn externalize(block: &mut Block, blobs: &dyn BlobStore) -> Result<(), StoreError> {
    match block {
        Block::User(user) => externalize_content(&mut user.content, blobs).await,
        Block::Steer(steer) => externalize_content(&mut steer.content, blobs).await,
        Block::ToolCall(call) => {
            for part in &mut call.output {
                let (ToolResultContentBlock::Image { source }
                | ToolResultContentBlock::Video { source }) = part
                else {
                    continue;
                };
                if let ToolMediaSource::Binary { data, media_type } = source {
                    let blob = blobs.put(data.clone()).await?;
                    *source = ToolMediaSource::Ref {
                        r#ref: blob,
                        media_type: std::mem::take(media_type),
                    };
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn externalize_content(
    content: &mut [UserContentBlock],
    blobs: &dyn BlobStore,
) -> Result<(), StoreError> {
    for part in content {
        match part {
            UserContentBlock::Image { source } | UserContentBlock::Video { source } => {
                if let MediaSource::Binary { data, media_type } = source {
                    let blob = blobs.put(data.clone()).await?;
                    *source = MediaSource::Ref {
                        r#ref: blob,
                        media_type: std::mem::take(media_type),
                    };
                }
            }
            UserContentBlock::Document { source } => {
                if let DocumentSource::Binary {
                    data,
                    media_type,
                    file_name,
                } = source
                {
                    let blob = blobs.put(data.clone()).await?;
                    *source = DocumentSource::Ref {
                        r#ref: blob,
                        media_type: std::mem::take(media_type),
                        file_name: std::mem::take(file_name),
                    };
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Puts the bytes of `block`'s media references back from `blobs`. A
/// reference whose blob is gone becomes the text `[missing <kind> blob
/// <ref>]`, so a session loaded for inference goes on.
pub async fn rehydrate(block: &mut Block, blobs: &dyn BlobStore) -> Result<(), StoreError> {
    match block {
        Block::User(user) => rehydrate_content(&mut user.content, blobs).await,
        Block::Steer(steer) => rehydrate_content(&mut steer.content, blobs).await,
        Block::ToolCall(call) => {
            for part in &mut call.output {
                let restored = match part {
                    ToolResultContentBlock::Image {
                        source: ToolMediaSource::Ref { r#ref, media_type },
                    } => blobs.get(r#ref).await?.map_or_else(
                        || tool_missing("image", r#ref),
                        |data| ToolResultContentBlock::Image {
                            source: ToolMediaSource::Binary {
                                data,
                                media_type: media_type.clone(),
                            },
                        },
                    ),
                    ToolResultContentBlock::Video {
                        source: ToolMediaSource::Ref { r#ref, media_type },
                    } => blobs.get(r#ref).await?.map_or_else(
                        || tool_missing("video", r#ref),
                        |data| ToolResultContentBlock::Video {
                            source: ToolMediaSource::Binary {
                                data,
                                media_type: media_type.clone(),
                            },
                        },
                    ),
                    _ => continue,
                };
                *part = restored;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn rehydrate_content(
    content: &mut [UserContentBlock],
    blobs: &dyn BlobStore,
) -> Result<(), StoreError> {
    for part in content {
        let restored = match part {
            UserContentBlock::Image {
                source: MediaSource::Ref { r#ref, media_type },
            } => blobs.get(r#ref).await?.map_or_else(
                || missing_text("image", r#ref),
                |data| UserContentBlock::Image {
                    source: MediaSource::Binary {
                        data,
                        media_type: media_type.clone(),
                    },
                },
            ),
            UserContentBlock::Video {
                source: MediaSource::Ref { r#ref, media_type },
            } => blobs.get(r#ref).await?.map_or_else(
                || missing_text("video", r#ref),
                |data| UserContentBlock::Video {
                    source: MediaSource::Binary {
                        data,
                        media_type: media_type.clone(),
                    },
                },
            ),
            UserContentBlock::Document {
                source:
                    DocumentSource::Ref {
                        r#ref,
                        media_type,
                        file_name,
                    },
            } => blobs.get(r#ref).await?.map_or_else(
                || missing_text("document", r#ref),
                |data| UserContentBlock::Document {
                    source: DocumentSource::Binary {
                        data,
                        media_type: media_type.clone(),
                        file_name: file_name.clone(),
                    },
                },
            ),
            _ => continue,
        };
        *part = restored;
    }
    Ok(())
}

fn missing(kind: &str, blob: &BlobRef) -> String {
    format!("[missing {kind} blob {blob}]")
}

fn missing_text(kind: &str, blob: &BlobRef) -> UserContentBlock {
    UserContentBlock::Text {
        text: missing(kind, blob),
    }
}

fn tool_missing(kind: &str, blob: &BlobRef) -> ToolResultContentBlock {
    ToolResultContentBlock::Text {
        text: missing(kind, blob),
    }
}

/// Moves the inline media of the blocks a frame carries into `blobs`: a root
/// or subagent transcript's blocks, or the whole blocks its patches put in.
/// The conversation socket sends every transcript frame this way.
pub async fn externalize_frame(
    frame: &mut ServerFrame,
    blobs: &dyn BlobStore,
) -> Result<(), StoreError> {
    match frame {
        ServerFrame::TranscriptReset { blocks, .. }
        | ServerFrame::SubagentTranscriptReset { blocks, .. } => {
            for block in blocks {
                externalize(block, blobs).await?;
            }
        }
        ServerFrame::TranscriptPatch { patches, .. }
        | ServerFrame::SubagentTranscriptPatch { patches, .. } => {
            for patch in patches {
                match patch {
                    TranscriptPatch::Add { value, .. }
                    | TranscriptPatch::ReplaceBlock { value, .. } => {
                        externalize(value, blobs).await?;
                    }
                    TranscriptPatch::Replace { value } => {
                        for block in value {
                            externalize(block, blobs).await?;
                        }
                    }
                    TranscriptPatch::Remove { .. } | TranscriptPatch::AppendText { .. } => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}
