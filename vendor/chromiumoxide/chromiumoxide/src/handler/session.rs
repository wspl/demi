use chromiumoxide_cdp::cdp::browser_protocol::target::{SessionId, TargetId};

/// Represents a Session within the cpd.
#[derive(Debug, Clone)]
pub struct Session {
    /// Identifier for this session.
    id: SessionId,
    /// The identifier of the target this session is attached to.
    target_id: TargetId,
    parent: Option<SessionId>,
}
impl Session {
    pub fn new(id: SessionId, target_id: TargetId, parent: Option<SessionId>) -> Self {
        Self {
            id,
            target_id,
            parent,
        }
    }

    pub fn parent(&self) -> Option<&SessionId> {
        self.parent.as_ref()
    }

    pub fn session_id(&self) -> &SessionId {
        &self.id
    }

    pub fn target_id(&self) -> &TargetId {
        &self.target_id
    }
}
