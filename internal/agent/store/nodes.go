package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// NodeRecord holds identity and relationship, never runtime state.
// The root has no parent. Returned records are owned snapshots.
type NodeRecord struct {
	ID core.NodeID
	// Number is the model-facing agent number: zero for the root.
	Number uint64
	Parent *core.NodeID
	// Description is a short title, empty for the root.
	Description string
	// Profile is absent for the root and inherited setups.
	Profile *string
	// Round starts at one and increases on each resume.
	Round             uint64
	StartedAt         core.Timestamp
	CanSpawnSubagents bool
	// Closed is nil while the node is live.
	Closed *NodeClose
	// Delivered reports whether this closed round reached its parent.
	Delivered bool
}

// RootRecord creates the conversation root, agent zero in its first round.
func RootRecord(id core.NodeID, now core.Timestamp) NodeRecord { panic("not written: a-store") }

// Job describes a child for subagent frames; the root has no job.
func (n NodeRecord) Job() *framewire.SubagentJob { panic("not written: a-store") }

// NodeClose describes how and when a node closed.
type NodeClose struct {
	Phase ClosePhase
	At    core.Timestamp
}

// ClosePhase is the phase a node closed in.
//
//sumtype:decl
type ClosePhase interface {
	closePhase()
	// JobPhase returns the phase a closed job shows.
	JobPhase() framewire.JobPhase
}

// Completed carries the child's bounded last assistant text.
type Completed struct{ Result string }

// Aborted marks a node that was aborted.
type Aborted struct{}

// Failed carries the failure's text.
type Failed struct{ Failure string }

func (*Completed) closePhase() {}
func (*Aborted) closePhase()   {}
func (*Failed) closePhase()    {}

// JobPhase returns the completed phase.
func (*Completed) JobPhase() framewire.JobPhase { panic("not written: a-store") }

// JobPhase returns the aborted phase.
func (*Aborted) JobPhase() framewire.JobPhase { panic("not written: a-store") }

// JobPhase returns the error phase.
func (*Failed) JobPhase() framewire.JobPhase { panic("not written: a-store") }
