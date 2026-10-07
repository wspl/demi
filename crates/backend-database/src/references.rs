//! What names a user's blobs (`storage.md` § Deleting a conversation): the
//! records a collection of the user's namespace reads before it removes a
//! blob. In the control database, the drafts of the user's conversations and
//! the plugins' values and Host directories; in each of the user's
//! conversation databases, its blocks with their media and edit copies, the
//! messages its checkpoints keep queued, the attachments the agent uploaded
//! and the records of its commands' outputs with their media. An upload
//! record names its blob too, but only a recent one keeps it: an upload
//! goes with its blob.

use std::collections::BTreeSet;

use demi_agent_store::CheckpointState;
use demi_agent_store::media::{block_blobs, content_references};
use demi_host_interface::MediumKept;
use demi_shared_types::{BlobRef, Block, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::{Connection, params};

use super::StorageError;
use super::columns::{decode, json};
use super::command_outputs::{self, OutputRow};
use super::control::ControlService;
use super::conversation_attachments;
use super::drafts::draft_blobs;
use super::plugin_values::plugin_blobs;

/// What the control database says about a user's blobs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlReferences {
    /// The blobs its records name, and those of the uploads recorded since
    /// the time asked about.
    pub blobs: BTreeSet<BlobRef>,
    /// The databases that may name more: the user's conversations', archived
    /// ones included, and those of the Forks the user has under way, whose
    /// destination is not published yet.
    pub databases: Vec<ConversationId>,
}

impl ControlService {
    /// The references of `owner`'s blobs in the control database, read in
    /// one transaction, with the blob of each upload recorded after
    /// `uploads_since`.
    pub async fn blob_references(
        &self,
        owner: UserId,
        uploads_since: Timestamp,
    ) -> Result<ControlReferences, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let mut references = ControlReferences::default();
            references.blobs.extend(draft_blobs(&transaction, &owner)?);
            references.blobs.extend(plugin_blobs(&transaction, &owner)?);
            {
                let mut uploads = transaction.prepare(
                    "SELECT sha256 FROM attachments WHERE user_id = ?1 AND created_at > ?2",
                )?;
                let mut rows =
                    uploads.query(params![owner.as_str(), uploads_since.as_millisecond()])?;
                while let Some(row) = rows.next()? {
                    references.blobs.insert(decode(
                        "attachments",
                        "sha256",
                        BlobRef::try_from(row.get::<_, String>(0)?),
                    )?);
                }
                let mut databases = transaction.prepare(
                    "SELECT id FROM conversations WHERE user_id = ?1
                     UNION SELECT f.id FROM conversation_fork_operations f
                       LEFT JOIN conversations c ON c.id = f.id
                       WHERE f.user_id = ?1 AND c.id IS NULL",
                )?;
                let mut rows = databases.query([owner.as_str()])?;
                while let Some(row) = rows.next()? {
                    references.databases.push(decode(
                        "conversations",
                        "id",
                        ConversationId::try_from(row.get::<_, String>(0)?),
                    )?);
                }
            }
            transaction.commit()?;
            Ok(references)
        })
        .await
    }

    /// Removes `owner`'s upload records of `blobs`, whose blobs a collection
    /// removed.
    pub async fn remove_uploads(
        &self,
        owner: UserId,
        blobs: Vec<BlobRef>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            {
                let mut remove = transaction
                    .prepare_cached("DELETE FROM attachments WHERE user_id = ?1 AND sha256 = ?2")?;
                for blob in &blobs {
                    remove.execute(params![owner.as_str(), blob.as_str()])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }
}

/// Every blob a conversation's database names, read on `connection` in the
/// read transaction it is given: each block's media and edit copies, every
/// row included, the media of the messages its checkpoints keep queued, the
/// agent's uploads, and each stored command output with its media.
pub fn conversation_blobs(connection: &Connection) -> Result<BTreeSet<BlobRef>, StorageError> {
    let mut blobs = BTreeSet::new();
    let mut blocks = connection.prepare("SELECT block FROM blocks")?;
    let mut rows = blocks.query([])?;
    while let Some(row) = rows.next()? {
        let block: Block = json("blocks", "block", &row.get::<_, String>(0)?)?;
        blobs.extend(block_blobs(&block).cloned());
    }
    let mut states = connection.prepare("SELECT state FROM nodes")?;
    let mut rows = states.query([])?;
    while let Some(row) = rows.next()? {
        let state: CheckpointState = json("nodes", "state", &row.get::<_, String>(0)?)?;
        for queued in &state.queue {
            blobs.extend(content_references(&queued.content).cloned());
        }
    }
    blobs.extend(
        conversation_attachments::all(connection)?
            .into_iter()
            .map(|attachment| attachment.blob),
    );
    for output in command_outputs::all(connection)? {
        if let OutputRow::Stored { blob, media, .. } = output.output {
            blobs.insert(blob);
            blobs.extend(media.into_iter().filter_map(|medium| match medium.kept {
                MediumKept::Stored { blob } => Some(blob),
                MediumKept::Missing { .. } => None,
            }));
        }
    }
    Ok(blobs)
}
