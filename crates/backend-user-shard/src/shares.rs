//! Shares on the owner's shard.
//!
//! The owner makes a share, makes a conversation's share's list again and
//! removes a share. Each is announced on the owner's pages through the
//! conversation's summary, which lists its shares.
//!
//! A share's reader reads what its list holds, by the share's record, which the
//! edge found by the token. A read reaches the list alone: no Host, no live
//! session.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use demi_core_blobs::ObjectError;
use demi_core_store::StorageError;
use demi_core_store::{conversation_attachments::{AttachmentNumber, AttachmentRow}};
use demi_backend_database::shares::{Added, ShareRecord};
use demi_core_store::shares::{self, ShareCounts};
use demi_core_store::{tree::{self, BlockRead, PageQuery, PageRead, Transcript}};
use demi_core_sync::Part;
use demi_shared_types::{BlobRef, BlockId};
use demi_web_api_protocol::ids::{ConversationId, ShareToken};
use demi_web_api_protocol::shares::{ShareTarget, ShareTranscriptQuery};
use rand::TryRngCore as _;
use rand::rngs::OsRng;

use demi_core::shard::Shard;

use crate::server_starter::ServerCore;

/// Why the owner's request about a share was refused.
#[derive(Debug, thiserror::Error)]
pub enum ShareRefusal {
    #[error("No such conversation")]
    NoConversation,
    #[error("No such share")]
    NoShare,
    #[error("Share the text that ends the agent's reply to a message")]
    TargetInvalid,
    #[error("No such reply block")]
    NoReplyBlock,
    #[error("Only a conversation's share is taken again")]
    NotUpdatable,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// A new share's token: 128 random bits from the operating system's
/// generator, in URL-safe base64.
fn new_token() -> ShareToken {
    let mut bits = [0_u8; 16];
    OsRng
        .try_fill_bytes(&mut bits)
        .expect("the operating system's random generator works");
    ShareToken::try_from(URL_SAFE_NO_PAD.encode(bits)).expect("16 bytes encode to a nonempty token")
}

/// The shares of the user's conversations, on the user's core.
#[expect(
    async_fn_in_trait,
    reason = "a core runs on one thread: its futures are never sent to another"
)]
pub trait ConversationSharing {
    /// The conversation's share of `target`: the one it has, or a new one
    /// with its list made now. A request that races another for the same
    /// target answers the share the first added, and drops its own list.
    async fn share(
        &self,
        conversation: ConversationId,
        target: ShareTarget,
    ) -> Result<ShareRecord, ShareRefusal>;

    /// Makes the share's list again under the same token: the
    /// conversation as it is now, with its title now.
    async fn update_share(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<ShareRecord, ShareRefusal>;

    /// Removes the share: its link answers as one that never existed from
    /// the record's removal on, and its list goes after it.
    async fn stop_sharing(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<(), ShareRefusal>;

    /// The share `token` of the user's conversation `conversation`.
    async fn owned_share(
        &self,
        conversation: &ConversationId,
        token: &ShareToken,
    ) -> Result<ShareRecord, ShareRefusal>;

    /// Makes the list of `target` under `token`, in place of one it had.
    async fn make_list(
        &self,
        conversation: &ConversationId,
        token: &ShareToken,
        target: &ShareTarget,
    ) -> Result<ShareCounts, ShareRefusal>;

    /// Removes a list no record names any more. One that stays costs
    /// storage only: nothing reads the rows of a token without its record.
    async fn drop_list(&self, conversation: &ConversationId, token: ShareToken);

    /// A page of what the share's list holds; none when the conversation
    /// has no database.
    async fn share_page(
        &self,
        share: &ShareRecord,
        query: ShareTranscriptQuery,
    ) -> Result<Option<PageRead>, StorageError>;

    /// A block the share's list holds, whole.
    async fn share_block(&self, share: &ShareRecord, block: BlockId) -> Result<Option<BlockRead>, StorageError>;

    /// The attachment `number` when the share's text names it.
    async fn shared_attachment(
        &self,
        share: &ShareRecord,
        number: AttachmentNumber,
    ) -> Result<Option<AttachmentRow>, StorageError>;

    /// Whether the share shows the reply block `<tag id="id" />`: the block
    /// a reply block's share is, or one a text of the list names
    /// (`shares::shows_block`).
    async fn shows_block(&self, share: &ShareRecord, tag: String, id: String) -> Result<bool, StorageError>;

    /// The bytes of `blob` when the share shows it.
    async fn share_blob(&self, share: &ShareRecord, blob: BlobRef) -> Result<Option<Bytes>, ShareBlobError>;
}

impl ConversationSharing for Shard {
    async fn share(
        &self,
        conversation: ConversationId,
        target: ShareTarget,
    ) -> Result<ShareRecord, ShareRefusal> {
        let store = self.store();
        let record = store.conversation(conversation)
            .await?
            .ok_or(ShareRefusal::NoConversation)?;
        if let Some(existing) = self.server_services().control.share_of(self.user().clone(), record.id.clone(), target.clone()).await? {
            return Ok(existing);
        }
        // A reply block is shared from the conversation a reply of which
        // names it, which its share goes with, and its plugin keeps it: one
        // it answers no facts of, on or off, is not the user's.
        if let ShareTarget::ReplyBlock { tag, id, .. } = &target {
            let (named_tag, named_id) = (tag.clone(), id.clone());
            let named = self.store().databases()
                .read(&record.id, move |connection| shares::names_block(connection, &named_tag, &named_id))
                .await?
                .unwrap_or(false);
            let held = [(tag.clone(), id.clone())].into_iter().collect();
            if !named || demi_core::conversation::reply_block_facts(self.plugins(), held).await.is_empty() {
                return Err(ShareRefusal::NoReplyBlock);
            }
        }
        let title = match &target {
            ShareTarget::ReplyBlock { title, .. } => Some(title.clone()),
            _ => record.title,
        };
        let token = new_token();
        let started = std::time::Instant::now();
        let counts = self.make_list(&record.id, &token, &target).await?;
        let made_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let share = ShareRecord {
            token: token.clone(),
            owner: self.user().clone(),
            conversation: record.id.clone(),
            target,
            title,
            taken_at: self.server_services().clock.now(),
        };
        match self.server_services().control.add_share(share).await? {
            Added::New(share) => {
                // The token is the link's secret, which no line holds.
                tracing::info!(
                    event = "share.created",
                    conversation = %record.id,
                    user = %self.user(),
                    target = match share.target {
                        ShareTarget::Conversation => "conversation",
                        ShareTarget::Answer { .. } => "answer",
                        ShareTarget::ReplyBlock { .. } => "reply_block",
                    },
                    blocks = counts.blocks,
                    nodes = counts.nodes,
                    attachments = counts.attachments,
                    ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(made_ms),
                    "a share was created"
                );
                self.mark(Part::Metadata(record.id));
                Ok(share)
            }
            // Another request shared it meanwhile, or the conversation went:
            // this list is no one's.
            Added::Existing(existing) => {
                self.drop_list(&record.id, token).await;
                Ok(existing)
            }
        }
    }

    async fn update_share(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<ShareRecord, ShareRefusal> {
        let share = self.owned_share(&conversation, &token).await?;
        if share.target != ShareTarget::Conversation {
            return Err(ShareRefusal::NotUpdatable);
        }
        let store = self.store();
        let record = store.conversation(share.conversation.clone())
            .await?
            .ok_or(ShareRefusal::NoConversation)?;
        let _ = self.make_list(&record.id, &token, &share.target).await?;
        let Some(share) = self.server_services().control
            .retake_share(token.clone(), record.title, self.server_services().clock.now())
            .await?
        else {
            // The owner stopped sharing meanwhile: this list is no one's.
            self.drop_list(&record.id, token).await;
            return Err(ShareRefusal::NoShare);
        };
        self.mark(Part::Metadata(record.id));
        Ok(share)
    }

    async fn stop_sharing(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<(), ShareRefusal> {
        let share = self.owned_share(&conversation, &token).await?;
        if !self
            .server_services()
            .control
            .remove_share(self.user().clone(), share.conversation.clone(), token.clone())
            .await?
        {
            return Err(ShareRefusal::NoShare);
        }
        self.mark(Part::Metadata(share.conversation.clone()));
        self.drop_list(&share.conversation, token).await;
        Ok(())
    }

    async fn owned_share(
        &self,
        conversation: &ConversationId,
        token: &ShareToken,
    ) -> Result<ShareRecord, ShareRefusal> {
        self.server_services()
            .control
            .share(token.clone())
            .await?
            .filter(|share| share.owner == *self.user() && share.conversation == *conversation)
            .ok_or(ShareRefusal::NoShare)
    }

    async fn make_list(
        &self,
        conversation: &ConversationId,
        token: &ShareToken,
        target: &ShareTarget,
    ) -> Result<ShareCounts, ShareRefusal> {
        let (token, target) = (token.clone(), target.clone());
        let taken = self.store().databases()
            .db(conversation)
            .call(move |connection| {
                let transaction = connection.transaction()?;
                let taken = match &target {
                    ShareTarget::Conversation => {
                        shares::take_conversation(&transaction, &token)?;
                        true
                    }
                    ShareTarget::Answer { block_id } => shares::take_answer(&transaction, &token, block_id)?,
                    // The record names the block; the list holds nothing.
                    ShareTarget::ReplyBlock { .. } => true,
                };
                let counts = shares::counts(&transaction, &token)?;
                transaction.commit()?;
                Ok((taken, counts))
            })
            .await?;
        if taken.0 { Ok(taken.1) } else { Err(ShareRefusal::TargetInvalid) }
    }

    async fn drop_list(&self, conversation: &ConversationId, token: ShareToken) {
        let removed = self.store().databases()
            .db(conversation)
            .call(move |connection| shares::remove(connection, &token))
            .await;
        if let Err(error) = removed {
            tracing::warn!(
                event = "share_list.left",
                conversation = %conversation,
                error = &error as &dyn std::error::Error,
                "a share's list that no record names stays in the conversation's database"
            );
        }
    }

    async fn share_page(
        &self,
        share: &ShareRecord,
        query: ShareTranscriptQuery,
    ) -> Result<Option<PageRead>, StorageError> {
        let transcript = Transcript::Shared {
            token: share.token.clone(),
        };
        let started = std::time::Instant::now();
        let read = self.store().databases()
            .read(&share.conversation, move |connection| {
                let page = PageQuery {
                    blocks: None,
                    before: query.before,
                    after: query.after,
                    around: query.around.as_ref(),
                    edge: query.edge.as_ref(),
                };
                tree::read_page(connection, &transcript, page)
            })
            .await;
        if let Ok(read) = &read {
            tracing::debug!(
                event = "share.read",
                conversation = %share.conversation,
                blocks = match read {
                    Some(PageRead::Page { blocks, .. }) => blocks.len(),
                    _ => 0,
                },
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "a page of a share was read"
            );
        }
        read
    }

    async fn share_block(&self, share: &ShareRecord, block: BlockId) -> Result<Option<BlockRead>, StorageError> {
        let transcript = Transcript::Shared {
            token: share.token.clone(),
        };
        self.store().databases()
            .read(&share.conversation, move |connection| {
                tree::read_block(connection, &transcript, &block)
            })
            .await
    }

    async fn shared_attachment(
        &self,
        share: &ShareRecord,
        number: AttachmentNumber,
    ) -> Result<Option<AttachmentRow>, StorageError> {
        let token = share.token.clone();
        Ok(self.store().databases()
            .read(&share.conversation, move |connection| {
                shares::shown_attachment(connection, &token, number)
            })
            .await?
            .flatten())
    }

    async fn shows_block(&self, share: &ShareRecord, tag: String, id: String) -> Result<bool, StorageError> {
        if let ShareTarget::ReplyBlock { tag: shared_tag, id: shared_id, .. } = &share.target {
            return Ok(*shared_tag == tag && *shared_id == id);
        }
        let token = share.token.clone();
        Ok(self.store().databases()
            .read(&share.conversation, move |connection| shares::shows_block(connection, &token, &tag, &id))
            .await?
            .unwrap_or(false))
    }

    async fn share_blob(&self, share: &ShareRecord, blob: BlobRef) -> Result<Option<Bytes>, ShareBlobError> {
        let token = share.token.clone();
        let wanted = blob.clone();
        let shown = self.store().databases()
            .read(&share.conversation, move |connection| {
                shares::shows_blob(connection, &token, &wanted)
            })
            .await?
            .unwrap_or(false);
        if !shown {
            return Ok(None);
        }
        Ok(self.server_services().blobs.for_user(&share.owner).get(&blob).await?)
    }
}

/// Why a share's blob could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ShareBlobError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Objects(#[from] ObjectError),
}
