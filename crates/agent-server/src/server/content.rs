//! A frame's content as the session receives it.
//!
//! Text and references are as they are. Uploads and remote files are as the
//! backend resolved them, with the bytes of the media the backend read. In an
//! edit, the references to the files the edited message holds are resolved by
//! the session from that message. No frame carries bytes.

use demi_agent_session::{Edit, EditComment, EditContent};
use demi_agent_store::media::HeldMedia;
use demi_conversation_lane_protocol::{AgentCode, ClientComment, ClientContent, Refusal};
use demi_shared_types::{MessageComment, UserContentBlock, is_blank};
use futures_util::future::LocalBoxFuture;

/// A file a message's content refers to, which only the backend can
/// resolve: an upload it holds or a file on a paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileReference {
    /// An upload by its attachment id, written under `file_name`.
    Upload {
        r#ref: String,
        file_name: String,
    },
    RemoteFile {
        device_id: String,
        path: String,
    },
}

/// Where the backend resolves the files of one frame's content
/// (`crates/backend-host-access/src/uploads.rs`).
///
/// - An upload is written to the conversation's Host and becomes its native
///   media block, when it has one, then its attachment record, or the text that
///   says it is unavailable.
/// - A remote file becomes its reference once its device may be read.
pub trait ContentResolver {
    /// The files `files` stand for, in the order given. All of one frame's
    /// references are resolved together, so that none is granted unless all
    /// can be; a failure refuses the frame.
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<ResolvedFiles, ContentError>>;
}

/// A frame's files as the backend resolved them: the blocks each reference
/// stands for, in order, with their media by reference, and the bytes of
/// those media the backend read, which the session holds.
#[derive(Debug, Default)]
pub struct ResolvedFiles {
    pub blocks: Vec<Vec<UserContentBlock>>,
    pub media: HeldMedia,
}

/// Why a frame's files could not be resolved; the frame is refused with it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ContentError {
    pub message: String,
    /// The code the refusal carries, such as `frame_delivery_failed`.
    pub code: Option<String>,
}

impl ContentError {
    /// The refusal of the send, the steer or the edit whose files these
    /// are, as the page words it: the code the error has, else
    /// `frame_failed`.
    pub(crate) fn refusal(&self) -> Refusal {
        let code = self.code.clone().unwrap_or_else(|| AgentCode::FrameFailed.to_string());
        Refusal::new(code, &self.message)
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: Some(AgentCode::FrameFailed.to_string()),
        }
    }
}

/// Resolves a `send`'s or a `steer`'s content and its comments', with the
/// bytes of their media. Such content never refers to what an edited message
/// holds: the frame's decode refused it.
pub(crate) async fn resolve_message(
    resolver: &dyn ContentResolver,
    content: Vec<ClientContent>,
    comments: Vec<ClientComment>,
) -> Result<(Vec<UserContentBlock>, Vec<MessageComment>, HeldMedia), ContentError> {
    let (content, comments, media) = resolve_edit(resolver, content, Some(comments)).await?;
    let sent = |parts: Vec<EditContent>| -> Result<Vec<UserContentBlock>, ContentError> {
        parts
            .into_iter()
            .map(|part| match part {
                EditContent::Content(block) => Ok(block),
                EditContent::KeptAttachment(_) | EditContent::KeptMedia(_) => Err(
                    ContentError::invalid("Only an edit refers to the files its message holds"),
                ),
            })
            .collect()
    };
    let comments = comments
        .unwrap_or_default()
        .into_iter()
        .map(|comment| {
            let parts = sent(comment.content.clone())?;
            Ok(comment.with_content::<demi_shared_types::Message>(parts))
        })
        .collect::<Result<_, ContentError>>()?;
    Ok((sent(content)?, comments, media))
}

/// Resolves content the backend hands the agent of its own, as a side
/// question's, with the bytes of its media, as a `send`'s.
pub async fn resolve_content(
    resolver: &dyn ContentResolver,
    content: Vec<ClientContent>,
) -> Result<(Vec<UserContentBlock>, HeldMedia), ContentError> {
    let (content, _, media) = resolve_message(resolver, content, Vec::new()).await?;
    Ok((content, media))
}

/// Resolves an edit's content and its comments', with the bytes of their new
/// media, keeping their references to the files the edited message holds for
/// the session. All their files are resolved together, so none is granted
/// unless all can be.
pub(crate) async fn resolve_edit(
    resolver: &dyn ContentResolver,
    content: Vec<ClientContent>,
    comments: Option<Vec<ClientComment>>,
) -> Result<(Vec<EditContent>, Option<Vec<EditComment>>, HeldMedia), ContentError> {
    let given = comments.is_some();
    let comments = comments.unwrap_or_default();
    let mut groups = vec![content];
    let mut anchors = Vec::with_capacity(comments.len());
    for mut comment in comments {
        groups.push(std::mem::take(&mut comment.content));
        anchors.push(comment);
    }
    let (mut parts, media) = resolve(resolver, groups).await?;
    let comments: Vec<EditComment> = anchors
        .into_iter()
        .zip(parts.drain(1..))
        .map(|(comment, content)| comment.with_content::<Edit>(content))
        .collect();
    let content = parts.pop().expect("the message's content is the first group");
    Ok((content, given.then_some(comments), media))
}

/// Resolves each group of content, a message's and each of its comments',
/// with all their file references in one call to the resolver.
async fn resolve(
    resolver: &dyn ContentResolver,
    groups: Vec<Vec<ClientContent>>,
) -> Result<(Vec<Vec<EditContent>>, HeldMedia), ContentError> {
    let files: Vec<FileReference> = groups
        .iter()
        .flatten()
        .filter_map(|part| match part {
            ClientContent::Upload { r#ref, file_name } => Some(FileReference::Upload {
                r#ref: r#ref.clone(),
                file_name: file_name.clone(),
            }),
            ClientContent::RemoteFile { device_id, path } => Some(FileReference::RemoteFile {
                device_id: device_id.clone(),
                path: path.clone(),
            }),
            _ => None,
        })
        .collect();
    let expected = files.len();
    let ResolvedFiles { blocks, media } = if files.is_empty() {
        ResolvedFiles::default()
    } else {
        resolver.resolve(files).await?
    };
    let mut resolved = blocks.into_iter();
    if resolved.len() != expected {
        return Err(ContentError::invalid(format!(
            "the backend resolved {} of {expected} file references",
            resolved.len()
        )));
    }
    let mut resolved_groups = Vec::with_capacity(groups.len());
    for group in groups {
        let mut parts = Vec::new();
        for part in group {
            match part {
                // The blank line an editor opens a message without text with is
                // no text; the request keeps it, so its digest stays as sent.
                ClientContent::Text { text } if is_blank(&text) => {}
                ClientContent::Text { text } => {
                    parts.push(EditContent::Content(UserContentBlock::Text { text }));
                }
                ClientContent::Reference { reference } => {
                    parts.push(EditContent::Content(UserContentBlock::Reference {
                        reference,
                    }));
                }
                ClientContent::Upload { .. } | ClientContent::RemoteFile { .. } => {
                    let blocks = resolved.next().expect("each file reference was resolved");
                    parts.extend(blocks.into_iter().map(EditContent::Content));
                }
                ClientContent::Media { media } => parts.push(EditContent::KeptMedia(media)),
                ClientContent::Attachment { id } => parts.push(EditContent::KeptAttachment(id)),
            }
        }
        resolved_groups.push(parts);
    }
    Ok((resolved_groups, media))
}
