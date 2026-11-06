//! The user's workspaces as the shard changes them: a deletion that deletes
//! the workspace's conversations first, and moves into a workspace, whose
//! changes the user's pages and the conversations' sockets hear of.

use demi_core_store::StorageError;
use demi_core_store::workspaces::{MoveRefusal, WorkspaceDeletion};
use demi_backend_page_sync::Part;
use demi_web_api_protocol::ids::{ConversationId, ProjectId, WorkspaceId};

use super::Shard;

/// Why a workspace's deletion was refused.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceRefusal {
    #[error("No such workspace")]
    NotFound,
    /// It is the user's last workspace, which stays.
    #[error("Your last workspace stays")]
    Last,
    /// A conversation moved in or was created while its conversations were
    /// deleted.
    #[error("A conversation came into the workspace meanwhile; try again")]
    Changed,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Shard {
    /// Deletes the user's workspace `id`: each of its conversations, archived
    /// ones included, as a deletion of the conversation deletes it, then its
    /// projects and the workspace. The last workspace is refused before
    /// anything is deleted.
    pub async fn delete_workspace(&self, id: &WorkspaceId) -> Result<(), WorkspaceRefusal> {
        let _one_at_a_time = self.workspace_deletions().acquire().await;
        let store = self.store();
        let workspaces = store.workspaces().await?;
        if !workspaces.iter().any(|workspace| workspace.id == *id) {
            return Err(WorkspaceRefusal::NotFound);
        }
        if workspaces.len() == 1 {
            return Err(WorkspaceRefusal::Last);
        }
        let conversations = store.workspace_conversations(id.clone())
            .await?
            .ok_or(WorkspaceRefusal::NotFound)?;
        // One moved to another workspace meanwhile stays.
        for conversation in &conversations {
            self.delete_conversation_in(conversation, Some(id)).await?;
        }
        match store.delete_workspace(id.clone()).await? {
            WorkspaceDeletion::Deleted => {}
            WorkspaceDeletion::Missing => return Err(WorkspaceRefusal::NotFound),
            WorkspaceDeletion::Last => return Err(WorkspaceRefusal::Last),
            WorkspaceDeletion::Changed => return Err(WorkspaceRefusal::Changed),
        }
        self.mark(Part::Workspaces);
        self.mark(Part::Projects);
        Ok(())
    }

    /// Moves the user's `conversations` and `projects` into the user's
    /// workspace `id` (`ControlService::move_to_workspace`).
    pub async fn move_to_workspace(
        &self,
        id: &WorkspaceId,
        conversations: Vec<ConversationId>,
        projects: Vec<ProjectId>,
    ) -> Result<Result<(), MoveRefusal>, StorageError> {
        let moved = match self
            .store().move_to_workspace(id.clone(), conversations, projects)
            .await?
        {
            Ok(moved) => moved,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if moved.projects {
            self.mark(Part::Projects);
        }
        // The pages of every conversation that shows a list a move changed,
        // the receiving project's others included, read it again.
        for conversation in &moved.saved {
            self.mark(Part::Metadata(conversation.clone()));
            if let Ok(sharers) = self.store().saved_sharers(conversation.clone()).await {
                for sharer in sharers.conversations {
                    self.mark(Part::Metadata(sharer));
                }
            }
        }
        for conversation in &moved.conversations {
            self.mark(Part::Conversation(conversation.clone()));
            // Its project may be another of the same directory now.
            self.project_files().release(conversation);
        }
        Ok(Ok(()))
    }
}
