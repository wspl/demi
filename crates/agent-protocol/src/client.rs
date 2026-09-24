//! What the browser sends (`runtime.md` § Client frames).

use std::future::Future;

use demi_core::{
    BlobRef, BlockId, CommandId, DocumentSource, MediaSource, ModelSelection, NodeId,
    OperationId, TurnId, UserContentBlock, is_blank,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::TranscriptVersion;

/// A frame the browser sends. The connection belongs to one conversation, so
/// no frame names a session or a working directory. `C` is the content of
/// `send`, `steer` and `edit_and_send`: [`ClientContent`] as the browser
/// sends it, or what the backend resolves it into before the session sees
/// it ([`ClientFrame::map_content`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ClientFrame<C = ClientContent>
where
    // garde's derive validates the content, so content of every kind is
    // itself validatable.
    C: garde::Validate<Context = ()>,
{
    /// Attach this connection to the conversation's tree, restoring it when
    /// it is not live, and align the tree's model with `model`.
    Open {
        #[garde(dive)]
        model: ModelSelection,
    },
    /// Submit a message; its id becomes its turn's id and its queue entry's.
    Send {
        #[garde(skip)]
        message_id: TurnId,
        #[garde(dive)]
        content: Vec<C>,
    },
    /// Replace a user message and everything after it.
    EditAndSend {
        #[garde(dive)]
        request: EditRequest<C>,
    },
    /// Add input to the running turn; its id is its `steer` block's.
    Steer {
        #[garde(skip)]
        steer_id: BlockId,
        #[garde(dive)]
        content: Vec<C>,
    },
    CancelPendingSteer {
        #[garde(skip)]
        steer_id: BlockId,
    },
    DequeueMessage {
        #[garde(skip)]
        message_id: TurnId,
    },
    /// Move a queued message to the front, so that it runs next.
    SendQueuedMessage {
        #[garde(skip)]
        message_id: TurnId,
    },
    /// Turn a queued message into a steer of the running turn.
    SteerQueuedMessage {
        #[garde(skip)]
        message_id: TurnId,
        #[garde(skip)]
        steer_id: BlockId,
    },
    ClearMessageQueue {},
    /// Change the model selection.
    SetProvider {
        #[garde(dive)]
        model: ModelSelection,
        /// When the switch lands; the next action when absent.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ModelSwitchApply")]
        #[garde(skip)]
        apply: Option<ModelSwitchApply>,
    },
    /// Stop one thing (`runtime.md` § Stop).
    Abort {},
    AbortSubagents {},
    /// Stop one live subagent with its subtree.
    AbortSubagent {
        #[garde(skip)]
        subagent_id: NodeId,
    },
    Retry {},
    Resume {},
    Compact {},
    /// Write stdin to a running command.
    ShellWrite {
        #[garde(skip)]
        command_id: CommandId,
        #[garde(skip)]
        stdin: String,
    },
    /// Stop a running command.
    ShellAbort {
        #[garde(skip)]
        command_id: CommandId,
    },
    /// Ask for a fresh transcript, after a gap in the patch revisions.
    SyncTranscript {},
    /// Dispose the tree.
    Close {},
}

/// Which frame a frame is: its `type`, which a `rejected` frame names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientFrameKind {
    Open,
    Send,
    EditAndSend,
    Steer,
    CancelPendingSteer,
    DequeueMessage,
    SendQueuedMessage,
    SteerQueuedMessage,
    ClearMessageQueue,
    SetProvider,
    Abort,
    AbortSubagents,
    AbortSubagent,
    Retry,
    Resume,
    Compact,
    ShellWrite,
    ShellAbort,
    SyncTranscript,
    Close,
}

serde_plain::derive_display_from_serialize!(ClientFrameKind);
serde_plain::derive_fromstr_from_deserialize!(ClientFrameKind);

impl<C> ClientFrame<C>
where
    C: garde::Validate<Context = ()>,
{
    pub fn kind(&self) -> ClientFrameKind {
        match self {
            Self::Open { .. } => ClientFrameKind::Open,
            Self::Send { .. } => ClientFrameKind::Send,
            Self::EditAndSend { .. } => ClientFrameKind::EditAndSend,
            Self::Steer { .. } => ClientFrameKind::Steer,
            Self::CancelPendingSteer { .. } => ClientFrameKind::CancelPendingSteer,
            Self::DequeueMessage { .. } => ClientFrameKind::DequeueMessage,
            Self::SendQueuedMessage { .. } => ClientFrameKind::SendQueuedMessage,
            Self::SteerQueuedMessage { .. } => ClientFrameKind::SteerQueuedMessage,
            Self::ClearMessageQueue {} => ClientFrameKind::ClearMessageQueue,
            Self::SetProvider { .. } => ClientFrameKind::SetProvider,
            Self::Abort {} => ClientFrameKind::Abort,
            Self::AbortSubagents {} => ClientFrameKind::AbortSubagents,
            Self::AbortSubagent { .. } => ClientFrameKind::AbortSubagent,
            Self::Retry {} => ClientFrameKind::Retry,
            Self::Resume {} => ClientFrameKind::Resume,
            Self::Compact {} => ClientFrameKind::Compact,
            Self::ShellWrite { .. } => ClientFrameKind::ShellWrite,
            Self::ShellAbort { .. } => ClientFrameKind::ShellAbort,
            Self::SyncTranscript {} => ClientFrameKind::SyncTranscript,
            Self::Close {} => ClientFrameKind::Close,
        }
    }

    /// The frame with the content of `send`, `steer` or `edit_and_send`
    /// replaced by what `resolve` makes of it, such as the attachment records
    /// of uploads written to the Host. Every other frame is unchanged and
    /// never calls `resolve`.
    pub async fn map_content<D, E, F, Fut>(self, resolve: F) -> Result<ClientFrame<D>, E>
    where
        D: garde::Validate<Context = ()>,
        F: FnOnce(Vec<C>) -> Fut,
        Fut: Future<Output = Result<Vec<D>, E>>,
    {
        let frame = match self {
            Self::Open { model } => ClientFrame::Open { model },
            Self::Send {
                message_id,
                content,
            } => ClientFrame::Send {
                message_id,
                content: resolve(content).await?,
            },
            Self::EditAndSend { request } => ClientFrame::EditAndSend {
                request: EditRequest {
                    operation_id: request.operation_id,
                    target_block_id: request.target_block_id,
                    version: request.version,
                    content: resolve(request.content).await?,
                },
            },
            Self::Steer { steer_id, content } => ClientFrame::Steer {
                steer_id,
                content: resolve(content).await?,
            },
            Self::CancelPendingSteer { steer_id } => ClientFrame::CancelPendingSteer { steer_id },
            Self::DequeueMessage { message_id } => ClientFrame::DequeueMessage { message_id },
            Self::SendQueuedMessage { message_id } => ClientFrame::SendQueuedMessage { message_id },
            Self::SteerQueuedMessage {
                message_id,
                steer_id,
            } => ClientFrame::SteerQueuedMessage {
                message_id,
                steer_id,
            },
            Self::ClearMessageQueue {} => ClientFrame::ClearMessageQueue {},
            Self::SetProvider { model, apply } => ClientFrame::SetProvider { model, apply },
            Self::Abort {} => ClientFrame::Abort {},
            Self::AbortSubagents {} => ClientFrame::AbortSubagents {},
            Self::AbortSubagent { subagent_id } => ClientFrame::AbortSubagent { subagent_id },
            Self::Retry {} => ClientFrame::Retry {},
            Self::Resume {} => ClientFrame::Resume {},
            Self::Compact {} => ClientFrame::Compact {},
            Self::ShellWrite { command_id, stdin } => ClientFrame::ShellWrite { command_id, stdin },
            Self::ShellAbort { command_id } => ClientFrame::ShellAbort { command_id },
            Self::SyncTranscript {} => ClientFrame::SyncTranscript {},
            Self::Close {} => ClientFrame::Close {},
        };
        Ok(frame)
    }
}

impl ClientFrame {
    /// The rules across a frame's content that its type does not hold: what
    /// the edited message already holds (`media`, `attachment`) appears only
    /// in `edit_and_send`, and an edit keeps text or a file.
    pub(crate) fn check_content(&self) -> Result<(), garde::Report> {
        let mut report = garde::Report::new();
        match self {
            Self::Send { content, .. } | Self::Steer { content, .. } => {
                for (index, part) in content.iter().enumerate() {
                    if part.is_kept_file() {
                        report.append(
                            garde::Path::new("content").join(index),
                            garde::Error::new("only an edit refers to the files its message holds"),
                        );
                    }
                }
            }
            Self::EditAndSend { request } => {
                let substance = request.content.iter().any(|part| match part {
                    ClientContent::Text { text } => !is_blank(text),
                    _ => true,
                });
                if !substance {
                    report.append(
                        garde::Path::new("request").join("content"),
                        garde::Error::new("a message must contain text or a file"),
                    );
                }
            }
            _ => {}
        }
        if report.is_empty() {
            Ok(())
        } else {
            Err(report)
        }
    }
}

/// A replacement of a user message and everything after it (`message-editing.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest<C = ClientContent>
where
    C: garde::Validate<Context = ()>,
{
    /// Chosen by the browser, so a repeated request is recognized.
    #[garde(skip)]
    pub operation_id: OperationId,
    /// The `user` block the edit replaces.
    #[garde(skip)]
    pub target_block_id: BlockId,
    /// The transcript version the editor showed; a stale one is refused.
    #[garde(dive)]
    pub version: TranscriptVersion,
    /// The complete replacement content.
    #[garde(length(min = 1), dive)]
    pub content: Vec<C>,
}

/// One part of the content the browser sends. Files are referred to, never
/// carried: a new file is an upload or a file on a paired device, and an edit
/// keeps what the edited message holds by reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ClientContent {
    Text {
        #[garde(skip)]
        text: String,
    },
    Reference {
        #[garde(skip)]
        reference: String,
    },
    /// A file the page uploaded (`web-api.md` § Uploads and media); the
    /// backend writes it to the Host before the session sees the content.
    Upload {
        /// The upload's attachment id.
        #[garde(length(chars, min = 1))]
        r#ref: String,
        /// The name the file is written under: no path separator, no NUL,
        /// and neither `.` nor `..`. The pattern has no lookaround, so that
        /// the browser's engine and Rust's read it alike.
        #[garde(
            length(chars, min = 1, max = 255),
            pattern(r"^(?:[^./\\\x00][^/\\\x00]*|\.[^./\\\x00][^/\\\x00]*|\.\.[^/\\\x00]+)$")
        )]
        file_name: String,
    },
    /// A file on a paired device, read at execution time
    /// (`web-api.md` § Device files and remote references).
    RemoteFile {
        #[garde(length(chars, min = 1))]
        device_id: String,
        /// Absolute.
        #[garde(length(chars, min = 1, max = 4096), pattern(r"^/[^\x00]*$"))]
        path: String,
    },
    /// In an edit: native media the edited message holds, by blob reference.
    Media {
        #[garde(dive)]
        media: MediaRef,
    },
    /// In an edit: an attachment record of the edited message, by the path
    /// the record holds (`message-editing.md` § Files the edit keeps).
    Attachment {
        #[garde(length(chars, min = 1, max = 4096))]
        path: String,
    },
}

impl ClientContent {
    /// Whether the part refers to a file the edited message holds.
    fn is_kept_file(&self) -> bool {
        matches!(self, Self::Media { .. } | Self::Attachment { .. })
    }
}

/// Native media an edited message holds, which its edit keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MediaRef {
    Image {
        #[garde(skip)]
        r#ref: BlobRef,
        #[garde(skip)]
        media_type: String,
    },
    Video {
        #[garde(skip)]
        r#ref: BlobRef,
        #[garde(skip)]
        media_type: String,
    },
    Document {
        #[garde(skip)]
        r#ref: BlobRef,
        #[garde(skip)]
        media_type: String,
        #[garde(length(chars, min = 1))]
        file_name: String,
    },
}

/// The content part the media is in the transcript: its block by reference.
impl From<MediaRef> for UserContentBlock {
    fn from(media: MediaRef) -> Self {
        match media {
            MediaRef::Image { r#ref, media_type } => UserContentBlock::Image {
                source: MediaSource::Ref { r#ref, media_type },
            },
            MediaRef::Video { r#ref, media_type } => UserContentBlock::Video {
                source: MediaSource::Ref { r#ref, media_type },
            },
            MediaRef::Document {
                r#ref,
                media_type,
                file_name,
            } => UserContentBlock::Document {
                source: DocumentSource::Ref {
                    r#ref,
                    media_type,
                    file_name,
                },
            },
        }
    }
}

/// When a model switch lands (`runtime.md` § Model switch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelSwitchApply {
    /// At the next continuation boundary, inside a running turn.
    Immediate,
    /// At the start of the next action.
    NextTurn,
}

serde_plain::derive_display_from_serialize!(ModelSwitchApply);
serde_plain::derive_fromstr_from_deserialize!(ModelSwitchApply);
