//! What the web app sends.

use demi_shared_types::{BlobRef, BlockId, CommandId, MessageComment, NodeId, OperationId, TurnId, is_blank};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::TranscriptVersion;

/// A frame the web app sends. The connection belongs to one conversation, so
/// no frame names a session or a working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ClientFrame {
    /// Attach this connection to the conversation's tree, restoring it when
    /// it is not live with the model selection the conversation's record
    /// holds; no frame names a model. `from` and `edge` name the root's
    /// blocks the page holds, so the reset sends none before them.
    Open {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        from: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        edge: Option<BlockId>,
    },
    /// Submit a message; its id becomes its turn's id and its queue entry's.
    Send {
        #[garde(skip)]
        message_id: TurnId,
        #[garde(dive)]
        content: Vec<ClientContent>,
        /// The comments the message carries, which may make it without
        /// content; omitted when there are none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[garde(dive)]
        comments: Vec<MessageComment>,
    },
    /// Continue from a new version of a user message.
    EditAndSend {
        #[garde(dive)]
        request: EditRequest,
    },
    /// Continue from a new version of a message's answer; answered by an
    /// `edit_result`, as an edit is.
    Regenerate {
        #[garde(dive)]
        request: RegenerateRequest,
    },
    /// Make the `to`th version at the block's place current, from 1, against
    /// the transcript `version` the page showed. The rewrite's patches
    /// answer it; a refusal answers `rejected`.
    SwitchVersion {
        #[garde(skip)]
        block_id: BlockId,
        #[garde(range(min = 1))]
        to: u32,
        #[garde(dive)]
        version: TranscriptVersion,
    },
    /// Add input to the running turn; its id is its `steer` block's.
    Steer {
        #[garde(skip)]
        steer_id: BlockId,
        #[garde(dive)]
        content: Vec<ClientContent>,
        /// The comments the steer carries, as a message's; omitted when
        /// there are none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[garde(dive)]
        comments: Vec<MessageComment>,
    },
    /// Send a message saved for later as the composer would, as the message
    /// `turn_id`, and remove it from the list in the same step. The backend handles it;
    /// `turn_id`, the page's id for this send, becomes its turn's or its steer's, never
    /// the saved message's own.
    SendSaved {
        #[garde(length(min = 1))]
        id: String,
        #[garde(skip)]
        turn_id: TurnId,
        #[garde(skip)]
        delivery: SavedDelivery,
    },
    CancelPendingSteer {
        #[garde(skip)]
        steer_id: BlockId,
    },
    /// Deliver a pending steer now.
    SteerNow {
        #[garde(skip)]
        steer_id: BlockId,
    },
    DequeueMessage {
        #[garde(skip)]
        message_id: TurnId,
    },
    /// Send a queued message now: while a turn runs, the turn ends at once
    /// and the message runs next; otherwise it moves to the front of the
    /// queue.
    SendQueuedMessage {
        #[garde(skip)]
        message_id: TurnId,
    },
    ClearMessageQueue {},
    /// Stop one thing.
    Abort {},
    AbortSubagents {},
    /// Stop one live subagent with its subtree.
    AbortSubagent {
        #[garde(skip)]
        subagent_id: NodeId,
    },
    /// The children whose transcripts this connection shows, the whole set
    /// each time it changes (`crates/agent-server/src/server/tree.rs`): a child it adds is
    /// answered with its `subagent_transcript_reset`, and only the children
    /// it names stream their transcripts to it. Sent before `open`, it names
    /// the children the open's handshake sends.
    ShowSubagents {
        #[garde(skip)]
        ids: Vec<NodeId>,
    },
    Retry {},
    Resume {},
    /// Compact the context of `node`, the root when absent: a subagent's
    /// context card compacts that child.
    Compact {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        node: Option<NodeId>,
    },
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
    /// Ask for a fresh transcript, after a gap in the patch revisions, from
    /// the page's blocks as `open` names them.
    SyncTranscript {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        from: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        edge: Option<BlockId>,
    },
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
    Regenerate,
    SwitchVersion,
    Steer,
    SendSaved,
    CancelPendingSteer,
    SteerNow,
    DequeueMessage,
    SendQueuedMessage,
    ClearMessageQueue,
    Abort,
    AbortSubagents,
    AbortSubagent,
    ShowSubagents,
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

impl ClientFrame {
    pub fn kind(&self) -> ClientFrameKind {
        match self {
            Self::Open { .. } => ClientFrameKind::Open,
            Self::Send { .. } => ClientFrameKind::Send,
            Self::EditAndSend { .. } => ClientFrameKind::EditAndSend,
            Self::Regenerate { .. } => ClientFrameKind::Regenerate,
            Self::SwitchVersion { .. } => ClientFrameKind::SwitchVersion,
            Self::Steer { .. } => ClientFrameKind::Steer,
            Self::SendSaved { .. } => ClientFrameKind::SendSaved,
            Self::CancelPendingSteer { .. } => ClientFrameKind::CancelPendingSteer,
            Self::SteerNow { .. } => ClientFrameKind::SteerNow,
            Self::DequeueMessage { .. } => ClientFrameKind::DequeueMessage,
            Self::SendQueuedMessage { .. } => ClientFrameKind::SendQueuedMessage,
            Self::ClearMessageQueue {} => ClientFrameKind::ClearMessageQueue,
            Self::Abort {} => ClientFrameKind::Abort,
            Self::AbortSubagents {} => ClientFrameKind::AbortSubagents,
            Self::AbortSubagent { .. } => ClientFrameKind::AbortSubagent,
            Self::ShowSubagents { .. } => ClientFrameKind::ShowSubagents,
            Self::Retry {} => ClientFrameKind::Retry,
            Self::Resume {} => ClientFrameKind::Resume,
            Self::Compact { .. } => ClientFrameKind::Compact,
            Self::ShellWrite { .. } => ClientFrameKind::ShellWrite,
            Self::ShellAbort { .. } => ClientFrameKind::ShellAbort,
            Self::SyncTranscript { .. } => ClientFrameKind::SyncTranscript,
            Self::Close {} => ClientFrameKind::Close,
        }
    }

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

/// How a message saved for later goes, the composer's choice: as the next
/// message while nothing runs, otherwise as a steer of the running turn or
/// into the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SavedDelivery {
    Message,
    Steer,
    Queue,
}

/// A replacement of a user message and everything after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    /// Chosen by the web app, so a repeated request is recognized.
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
    pub content: Vec<ClientContent>,
}

/// Regenerate of the answer to a user message: the message stays as it
/// is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegenerateRequest {
    /// Chosen by the web app, so a repeated request is recognized.
    #[garde(skip)]
    pub operation_id: OperationId,
    /// The `user` block whose answer gets a new version.
    #[garde(skip)]
    pub target_block_id: BlockId,
    /// The transcript version the page showed; a stale one is refused.
    #[garde(dive)]
    pub version: TranscriptVersion,
}

/// One part of the content the web app sends. Files are referred to, never
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
    /// A file the page uploaded; the backend writes it to the Host before the
    /// session sees the content.
    Upload {
        /// The upload's attachment id.
        #[garde(length(chars, min = 1))]
        r#ref: String,
        /// The name the file is written under: no path separator, no NUL,
        /// and neither `.` nor `..`. The pattern has no lookaround, so that
        /// a web browser's regex engine and Rust's read it alike.
        #[garde(
            length(chars, min = 1, max = 255),
            pattern(r"^(?:[^./\\\x00][^/\\\x00]*|\.[^./\\\x00][^/\\\x00]*|\.\.[^/\\\x00]+)$")
        )]
        file_name: String,
    },
    /// A file on a paired device, read at execution time.
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
    /// In an edit: an attachment record of the edited message, by its
    /// number, such as `a7`.
    Attachment {
        #[garde(length(chars, min = 1, max = 32))]
        id: String,
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

