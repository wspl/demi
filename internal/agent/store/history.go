package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"encoding/json"

	"github.com/wspl/demi/internal/core"
)

// CommandStateError explains why a command state is refused.
type CommandStateError struct{ Reason string }

// Error returns the refusal text.
func (e *CommandStateError) Error() string { panic("not written: a-store") }

// validateCommandStorageKey checks a node's logical command storage name.
func validateCommandStorageKey(key CommandStorageKey) error { panic("not written: a-store") }

// InitialCommandState returns the explicit empty version zero, current.
func InitialCommandState() CommandStateSnapshot { panic("not written: a-store") }

// CommandStateHistory holds a session's live command state and checkpoint dirtiness.
// The session owns and serializes its use; returned maps and snapshots are copies.
type CommandStateHistory struct{}

// NewCommandStateHistory creates a node's history, already held by its first checkpoint.
func NewCommandStateHistory() *CommandStateHistory { panic("not written: a-store") }

// RestoreCommandStateHistory checks unique revisions, empty version zero, the
// current version, and unique boundaries referring to existing versions.
// Invalid data is refused, never repaired.
func RestoreCommandStateHistory(snapshot CommandStateSnapshot) (*CommandStateHistory, error) {
	panic("not written: a-store")
}

// Revision returns the current version's revision.
func (h *CommandStateHistory) Revision() uint64 { panic("not written: a-store") }

// Values returns an owned copy of the current version's keys and values.
func (h *CommandStateHistory) Values() map[CommandStorageKey]json.RawMessage {
	panic("not written: a-store")
}

// Prepare returns the next version, or nil for canonically equal values.
// Values must be valid JSON; revision allocation is one past the largest used.
func (h *CommandStateHistory) Prepare(values map[CommandStorageKey]json.RawMessage) (*CommandVersion, error) {
	panic("not written: a-store")
}

// Accept makes a committed version current.
func (h *CommandStateHistory) Accept(version CommandVersion) { panic("not written: a-store") }

// Boundary returns the revision recorded at an edge, and whether it exists.
func (h *CommandStateHistory) Boundary(block core.BlockID, edge BoundaryEdge) (uint64, bool) {
	panic("not written: a-store")
}

// Select keeps the retained blocks' boundaries, selecting revision as current.
// With referencedOnly, it copies only the referenced versions, as a Fork does.
func (h *CommandStateHistory) Select(blocks []core.Block, revision uint64, referencedOnly bool) CommandStateSnapshot {
	panic("not written: a-store")
}

// Capture records revision at the edge. BeforeUser and AfterAssistant keep
// their first record; AfterBlock moves forward with its block.
func (h *CommandStateHistory) Capture(block core.BlockID, edge BoundaryEdge, revision uint64) {
	panic("not written: a-store")
}

// HasBoundary reports whether the edge has a recorded revision.
func (h *CommandStateHistory) HasBoundary(block core.BlockID, edge BoundaryEdge) bool {
	panic("not written: a-store")
}

// Snapshot returns the whole state, with pending current when present.
func (h *CommandStateHistory) Snapshot(pending *CommandVersion) CommandStateSnapshot {
	panic("not written: a-store")
}

// TakeUpdate returns changed state, or state carrying pending, and clears dirtiness.
func (h *CommandStateHistory) TakeUpdate(pending *CommandVersion) *CommandStateSnapshot {
	panic("not written: a-store")
}

// MarkDirty makes a failed save's next attempt carry command state again.
func (h *CommandStateHistory) MarkDirty() { panic("not written: a-store") }
