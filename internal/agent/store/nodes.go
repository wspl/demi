package store

import (
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// NodeRecord holds identity and relationship, never runtime state.
// The root has no parent. Returned records are owned snapshots.
type NodeRecord struct {
	// ID identifies the node.
	ID types.NodeID
	// Number is the model-facing agent number: zero for the root.
	Number uint64
	// Parent identifies the parent, or is nil for the root.
	Parent *types.NodeID
	// Description is a short title, empty for the root.
	Description string
	// Profile is absent for the root and inherited setups.
	Profile *string
	// Round starts at one and increases on each resume.
	Round uint64
	// StartedAt records when the current round started.
	StartedAt types.Timestamp
	// CanSpawnSubagents reports whether the node may create children.
	CanSpawnSubagents bool
	// Closed is nil while the node is live.
	Closed *NodeClose
	// Delivered reports whether this closed round reached its parent.
	Delivered bool
}

// RootRecord creates the conversation root, agent zero in its first round.
func RootRecord(id types.NodeID, now types.Timestamp) NodeRecord {
	return NodeRecord{ID: id, Round: 1, StartedAt: now, CanSpawnSubagents: true}
}

// Job describes a child for subagent frames, and false for the root, which has no job.
func (n NodeRecord) Job() (conversationproto.SubagentJob, bool) {
	if n.Parent == nil {
		return conversationproto.SubagentJob{}, false
	}
	job := conversationproto.SubagentJob{
		SubagentID:      n.ID,
		ParentSessionID: *n.Parent,
		Description:     n.Description,
		Profile:         n.Profile,
		Phase:           conversationproto.JobPhaseRunning,
		StartedAt:       n.StartedAt,
	}
	if n.Closed != nil {
		job.Phase = n.Closed.Phase.JobPhase()
		job.EndedAt = new(n.Closed.At)
		if completed, ok := n.Closed.Phase.(*Completed); ok {
			job.Result = new(completed.Result)
		}
	}
	if n.Profile != nil {
		job.Profile = new(*n.Profile)
	}
	return job, true
}

// NodeClose describes how and when a node closed.
type NodeClose struct {
	// Phase records the close outcome.
	Phase ClosePhase
	// At records when the node closed.
	At types.Timestamp
}

// ClosePhase is the phase a node closed in.
//
//sumtype:decl
type ClosePhase interface {
	closePhase()
	// JobPhase returns the phase a closed job shows.
	JobPhase() conversationproto.JobPhase
}

// Completed carries the child's bounded last assistant text.
type Completed struct {
	// Result holds the child's final bounded assistant text.
	Result string
}

// Aborted marks a node that was aborted.
type Aborted struct{}

// Failed carries the failure's text.
type Failed struct {
	// Failure holds the failure text.
	Failure string
}

func (*Completed) closePhase() {}
func (*Aborted) closePhase()   {}
func (*Failed) closePhase()    {}

// JobPhase returns the completed phase.
func (*Completed) JobPhase() conversationproto.JobPhase {
	return conversationproto.JobPhaseCompleted
}

// JobPhase returns the aborted phase.
func (*Aborted) JobPhase() conversationproto.JobPhase {
	return conversationproto.JobPhaseAborted
}

// JobPhase returns the error phase.
func (*Failed) JobPhase() conversationproto.JobPhase {
	return conversationproto.JobPhaseError
}
