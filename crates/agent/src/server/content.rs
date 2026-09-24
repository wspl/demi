//! A frame's content as the session receives it (`runtime.md` § Client
//! frames): text and references as they are, uploads and remote files as
//! the backend resolved them, and, in an edit, the files the edited message
//! already holds. The backend resolves the files; no frame carries bytes.

use demi_agent_protocol::ClientContent;
use demi_core::UserContentBlock;
use futures_util::future::LocalBoxFuture;

use crate::session::EditContent;

/// A file a message's content refers to, which only the backend can
/// resolve: an upload it holds, or a file on a paired device.
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
/// (`backend.md` § Media by reference): an upload is written to the
/// conversation's Host and becomes its native media block, when it has one,
/// then its attachment record, or the text that says it is unavailable; a
/// remote file becomes its reference once its device may be read.
pub trait ContentResolver {
    /// The blocks each reference stands for, in the order given. All of one
    /// frame's references are resolved together, so that none is granted
    /// unless all can be; a failure refuses the frame.
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<Vec<Vec<UserContentBlock>>, ContentError>>;
}

/// Why a frame's files could not be resolved; the frame is refused with it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ContentError {
    pub message: String,
    /// The code the backend's `error` frame carries, such as
    /// `frame_delivery_failed`.
    pub code: Option<String>,
}

impl ContentError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }
}

/// Resolves a `send`'s or a `steer`'s content. Such content never refers to
/// what an edited message holds: the frame's decode refused it.
pub(crate) async fn resolve_message(
    resolver: &dyn ContentResolver,
    content: Vec<ClientContent>,
) -> Result<Vec<UserContentBlock>, ContentError> {
    resolve(resolver, content)
        .await?
        .into_iter()
        .map(|part| match part {
            EditContent::Content(block) => Ok(block),
            EditContent::KeptAttachment(_) => Err(ContentError::invalid(
                "Only an edit refers to the files its message holds",
            )),
        })
        .collect()
}

/// Resolves an edit's content, keeping its references to the edited
/// message's attachment records for the session.
pub(crate) async fn resolve_edit(
    resolver: &dyn ContentResolver,
    content: Vec<ClientContent>,
) -> Result<Vec<EditContent>, ContentError> {
    resolve(resolver, content).await
}

async fn resolve(
    resolver: &dyn ContentResolver,
    content: Vec<ClientContent>,
) -> Result<Vec<EditContent>, ContentError> {
    let files: Vec<FileReference> = content
        .iter()
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
    let mut resolved = if files.is_empty() {
        Vec::new().into_iter()
    } else {
        resolver.resolve(files).await?.into_iter()
    };
    if resolved.len() != expected {
        return Err(ContentError::invalid(format!(
            "the backend resolved {} of {expected} file references",
            resolved.len()
        )));
    }
    let mut parts = Vec::new();
    for part in content {
        match part {
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
            ClientContent::Media { media } => {
                parts.push(EditContent::Content(media.into()));
            }
            ClientContent::Attachment { path } => parts.push(EditContent::KeptAttachment(path)),
        }
    }
    Ok(parts)
}
