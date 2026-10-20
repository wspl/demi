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
use demi_backend_blobs::ObjectError;
use demi_backend_database::StorageError;
use demi_backend_database::conversation_attachments::{AttachmentNumber, AttachmentRow};
use demi_backend_database::shares::{self, Added, ShareRecord};
use demi_backend_database::tree::{self, BlockRead, PageQuery, PageRead, Transcript};
use demi_backend_page_sync::Part;
use demi_shared_types::{BlobRef, BlockId};
use demi_web_api_protocol::ids::{ConversationId, ShareToken};
use demi_web_api_protocol::shares::{ShareTarget, ShareTranscriptQuery};
use rand::TryRngCore as _;
use rand::rngs::OsRng;

use crate::shard::Shard;

/// Why the owner's request about a share was refused.
#[derive(Debug, thiserror::Error)]
pub enum ShareRefusal {
    #[error("No such conversation")]
    NoConversation,
    #[error("No such share")]
    NoShare,
    #[error("Share the text that ends the agent's reply to a message")]
    TargetInvalid,
    #[error("An answer's share is not taken again")]
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

impl Shard {
    /// The conversation's share of `target`: the one it has, or a new one
    /// with its list made now. A request that races another for the same
    /// target answers the share the first added, and drops its own list.
    pub async fn share(
        &self,
        conversation: ConversationId,
        target: ShareTarget,
    ) -> Result<ShareRecord, ShareRefusal> {
        let control = &self.services().control;
        let record = control
            .conversation(conversation)
            .await?
            .filter(|record| record.owner == *self.user())
            .ok_or(ShareRefusal::NoConversation)?;
        if let Some(existing) = control.share_of(record.id.clone(), target.clone()).await? {
            return Ok(existing);
        }
        let token = new_token();
        self.make_list(&record.id, &token, &target).await?;
        let share = ShareRecord {
            token: token.clone(),
            owner: self.user().clone(),
            conversation: record.id.clone(),
            target,
            title: record.title,
            taken_at: self.services().clock.now(),
        };
        match control.add_share(share).await? {
            Added::New(share) => {
                self.mark(Part::Conversation(record.id));
                Ok(share)
            }
            // Another request shared it meanwhile, or the conversation went:
            // this list is no one's.
            Added::Existing(existing) => {
                self.drop_list(&record.id, token).await;
                Ok(existing)
            }
            Added::NoConversation => {
                self.drop_list(&record.id, token).await;
                Err(ShareRefusal::NoConversation)
            }
        }
    }

    /// Makes the share's list again under the same token: the
    /// conversation as it is now, with its title now.
    pub async fn update_share(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<ShareRecord, ShareRefusal> {
        let share = self.owned_share(&conversation, &token).await?;
        if share.target != ShareTarget::Conversation {
            return Err(ShareRefusal::NotUpdatable);
        }
        let control = &self.services().control;
        let record = control
            .conversation(share.conversation.clone())
            .await?
            .ok_or(ShareRefusal::NoConversation)?;
        self.make_list(&record.id, &token, &share.target).await?;
        let Some(share) = control
            .retake_share(token.clone(), record.title, self.services().clock.now())
            .await?
        else {
            // The owner stopped sharing meanwhile: this list is no one's.
            self.drop_list(&record.id, token).await;
            return Err(ShareRefusal::NoShare);
        };
        self.mark(Part::Conversation(record.id));
        Ok(share)
    }

    /// Removes the share: its link answers as one that never existed from
    /// the record's removal on, and its list goes after it.
    pub async fn stop_sharing(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<(), ShareRefusal> {
        let share = self.owned_share(&conversation, &token).await?;
        if !self
            .services()
            .control
            .remove_share(share.conversation.clone(), token.clone())
            .await?
        {
            return Err(ShareRefusal::NoShare);
        }
        self.mark(Part::Conversation(share.conversation.clone()));
        self.drop_list(&share.conversation, token).await;
        Ok(())
    }

    /// The share `token` of the user's conversation `conversation`.
    async fn owned_share(
        &self,
        conversation: &ConversationId,
        token: &ShareToken,
    ) -> Result<ShareRecord, ShareRefusal> {
        self.services()
            .control
            .share(token.clone())
            .await?
            .filter(|share| share.owner == *self.user() && share.conversation == *conversation)
            .ok_or(ShareRefusal::NoShare)
    }

    /// Makes the list of `target` under `token`, in place of one it had.
    async fn make_list(
        &self,
        conversation: &ConversationId,
        token: &ShareToken,
        target: &ShareTarget,
    ) -> Result<(), ShareRefusal> {
        let (token, target) = (token.clone(), target.clone());
        let taken = self
            .services()
            .conversations
            .db(conversation)
            .call(move |connection| {
                let transaction = connection.transaction()?;
                let taken = match &target {
                    ShareTarget::Conversation => {
                        shares::take_conversation(&transaction, &token)?;
                        true
                    }
                    ShareTarget::Answer { block_id } => shares::take_answer(&transaction, &token, block_id)?,
                };
                transaction.commit()?;
                Ok(taken)
            })
            .await?;
        if taken { Ok(()) } else { Err(ShareRefusal::TargetInvalid) }
    }

    /// Removes a list no record names any more. One that stays costs
    /// storage only: nothing reads the rows of a token without its record.
    async fn drop_list(&self, conversation: &ConversationId, token: ShareToken) {
        let removed = self
            .services()
            .conversations
            .db(conversation)
            .call(move |connection| shares::remove(connection, &token))
            .await;
        if let Err(error) = removed {
            tracing::warn!(
                conversation = %conversation,
                error = &error as &dyn std::error::Error,
                "a share's list that no record names stays in the conversation's database"
            );
        }
    }

    /// A page of what the share's list holds; none when the conversation
    /// has no database.
    pub async fn share_page(
        &self,
        share: &ShareRecord,
        query: ShareTranscriptQuery,
    ) -> Result<Option<PageRead>, StorageError> {
        let transcript = Transcript::Shared {
            token: share.token.clone(),
        };
        self.services()
            .conversations
            .read(&share.conversation, move |connection| {
                let page = PageQuery {
                    before: query.before,
                    after: query.after,
                    around: query.around.as_ref(),
                    edge: query.edge.as_ref(),
                };
                tree::read_page(connection, &transcript, page)
            })
            .await
    }

    /// A block the share's list holds, whole.
    pub async fn share_block(&self, share: &ShareRecord, block: BlockId) -> Result<Option<BlockRead>, StorageError> {
        let transcript = Transcript::Shared {
            token: share.token.clone(),
        };
        self.services()
            .conversations
            .read(&share.conversation, move |connection| {
                tree::read_block(connection, &transcript, &block)
            })
            .await
    }

    /// The attachment `number` when the share's text names it.
    pub async fn shared_attachment(
        &self,
        share: &ShareRecord,
        number: AttachmentNumber,
    ) -> Result<Option<AttachmentRow>, StorageError> {
        let token = share.token.clone();
        Ok(self
            .services()
            .conversations
            .read(&share.conversation, move |connection| {
                shares::shown_attachment(connection, &token, number)
            })
            .await?
            .flatten())
    }

    /// The bytes of `blob` when the share shows it.
    pub async fn share_blob(&self, share: &ShareRecord, blob: BlobRef) -> Result<Option<Bytes>, ShareBlobError> {
        let token = share.token.clone();
        let wanted = blob.clone();
        let shown = self
            .services()
            .conversations
            .read(&share.conversation, move |connection| {
                shares::shows_blob(connection, &token, &wanted)
            })
            .await?
            .unwrap_or(false);
        if !shown {
            return Ok(None);
        }
        Ok(self.services().blobs.for_user(&share.owner).get(&blob).await?)
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
