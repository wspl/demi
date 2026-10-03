package store

import (
	"bytes"
	"encoding/json"
	"fmt"
	"maps"
	"slices"
	"strings"

	"github.com/gowebpki/jcs"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// CommandStateError explains why a command state is refused.
type CommandStateError struct {
	// Reason is the command-state refusal text.
	Reason string
}

// Error returns the refusal text.
func (e *CommandStateError) Error() string { return e.Reason }

// validateCommandStorageKey checks a node's logical command storage name.
func validateCommandStorageKey(key CommandStorageKey) error {
	s := string(key)
	drive := len(s) >= 3 && (s[0] >= 'a' && s[0] <= 'z' || s[0] >= 'A' && s[0] <= 'Z') && s[1] == ':' &&
		(s[2] == '/' || s[2] == '\\')
	traverses := slices.Contains(strings.FieldsFunc(s, func(r rune) bool { return r == '/' || r == '\\' }), "..")
	if s == "" || strings.ContainsRune(s, 0) || strings.HasPrefix(s, "/") || drive || traverses {
		return &CommandStateError{
			Reason: fmt.Sprintf("command storage key %q is not a relative name without path traversal", s),
		}
	}
	return nil
}

// InitialCommandState returns the explicit empty version zero, current.
func InitialCommandState() CommandStateSnapshot {
	return CommandStateSnapshot{
		Versions:   []CommandVersion{{Values: map[CommandStorageKey]json.RawMessage{}}},
		Boundaries: []SessionBoundary{},
	}
}

type boundaryKey struct {
	block core.BlockID
	edge  BoundaryEdge
}

// CommandStateHistory holds a session's live command state and checkpoint dirtiness.
// The session owns and serializes its use; returned maps and snapshots are copies.
type CommandStateHistory struct {
	versions   map[uint64]map[CommandStorageKey]json.RawMessage
	boundaries map[boundaryKey]uint64
	current    uint64
	dirty      bool
}

// NewCommandStateHistory creates a node's history, already held by its first checkpoint.
func NewCommandStateHistory() *CommandStateHistory {
	return &CommandStateHistory{
		versions:   map[uint64]map[CommandStorageKey]json.RawMessage{0: {}},
		boundaries: map[boundaryKey]uint64{},
	}
}

// RestoreCommandStateHistory checks unique revisions, empty version zero, the
// current version, and unique boundaries referring to existing versions.
// Invalid data is refused, never repaired.
func RestoreCommandStateHistory(snapshot CommandStateSnapshot) (*CommandStateHistory, error) {
	refuse := func(reason string) (*CommandStateHistory, error) {
		return nil, &CommandStateError{Reason: "invalid command state: " + reason}
	}
	h := &CommandStateHistory{
		versions:   map[uint64]map[CommandStorageKey]json.RawMessage{},
		boundaries: map[boundaryKey]uint64{},
		current:    snapshot.Revision,
	}
	for _, version := range snapshot.Versions {
		if _, exists := h.versions[version.Revision]; exists {
			return refuse("a revision appears twice")
		}
		h.versions[version.Revision] = cloneValues(version.Values)
	}
	if values, exists := h.versions[0]; !exists || len(values) != 0 {
		return refuse("version 0 is missing or not empty")
	}
	if _, exists := h.versions[h.current]; !exists {
		return refuse("the current revision has no version")
	}
	for _, boundary := range snapshot.Boundaries {
		if _, exists := h.versions[boundary.CommandRevision]; !exists {
			return refuse("a boundary names a revision that has no version")
		}
		key := boundaryKey{boundary.BlockID, boundary.Edge}
		if _, exists := h.boundaries[key]; exists {
			return refuse("a block has two boundaries of one edge")
		}
		h.boundaries[key] = boundary.CommandRevision
	}
	return h, nil
}

// Revision returns the current version's revision.
func (h *CommandStateHistory) Revision() uint64 { return h.current }

// Values returns an owned copy of the current version's keys and values.
func (h *CommandStateHistory) Values() map[CommandStorageKey]json.RawMessage {
	return cloneValues(h.versions[h.current])
}

// Prepare returns the next version, or nil for canonically equal values.
// Values must be valid JSON; revision allocation is one past the largest used.
func (h *CommandStateHistory) Prepare(values map[CommandStorageKey]json.RawMessage) (*CommandVersion, error) {
	values = cloneValues(values)
	proposed, err := canonicalValues(values)
	if err != nil {
		return nil, fmt.Errorf("command state: %w", err)
	}
	current, err := canonicalValues(h.versions[h.current])
	if err != nil {
		return nil, fmt.Errorf("command state: %w", err)
	}
	if bytes.Equal(proposed, current) {
		return nil, nil
	}
	var largest uint64
	for revision := range h.versions {
		largest = max(largest, revision)
	}
	return &CommandVersion{Revision: largest + 1, Values: values}, nil
}

// Accept makes a committed version current.
func (h *CommandStateHistory) Accept(version CommandVersion) {
	h.current = version.Revision
	h.versions[version.Revision] = cloneValues(version.Values)
}

// Boundary returns the revision recorded at an edge, and whether it exists.
func (h *CommandStateHistory) Boundary(block core.BlockID, edge BoundaryEdge) (uint64, bool) {
	revision, ok := h.boundaries[boundaryKey{block, edge}]
	return revision, ok
}

// Select keeps the retained blocks' boundaries, selecting revision as current.
// With referencedOnly, it copies only the referenced versions, as a Fork does.
func (h *CommandStateHistory) Select(blocks []core.Block, revision uint64, referencedOnly bool) CommandStateSnapshot {
	retained := map[core.BlockID]bool{}
	for _, block := range blocks {
		retained[block.ID()] = true
	}
	snapshot := h.Snapshot(nil)
	snapshot.Revision = revision
	snapshot.Boundaries = slices.DeleteFunc(
		snapshot.Boundaries,
		func(b SessionBoundary) bool { return !retained[b.BlockID] },
	)
	if referencedOnly {
		referenced := map[uint64]bool{0: true, revision: true}
		for _, boundary := range snapshot.Boundaries {
			referenced[boundary.CommandRevision] = true
		}
		snapshot.Versions = slices.DeleteFunc(
			snapshot.Versions,
			func(v CommandVersion) bool { return !referenced[v.Revision] },
		)
	}
	return snapshot
}

// Capture records revision at the edge. BeforeUser and AfterAssistant keep
// their first record; AfterBlock moves forward with its block.
func (h *CommandStateHistory) Capture(block core.BlockID, edge BoundaryEdge, revision uint64) {
	key := boundaryKey{block, edge}
	if previous, exists := h.boundaries[key]; exists && (previous == revision || edge != AfterBlock) {
		return
	}
	h.boundaries[key] = revision
	h.dirty = true
}

// HasBoundary reports whether the edge has a recorded revision.
func (h *CommandStateHistory) HasBoundary(block core.BlockID, edge BoundaryEdge) bool {
	_, ok := h.Boundary(block, edge)
	return ok
}

// Snapshot returns the whole state, with pending current when present.
func (h *CommandStateHistory) Snapshot(pending *CommandVersion) CommandStateSnapshot {
	snapshot := CommandStateSnapshot{Revision: h.current, Versions: []CommandVersion{}, Boundaries: []SessionBoundary{}}
	for _, revision := range slices.Sorted(maps.Keys(h.versions)) {
		snapshot.Versions = append(
			snapshot.Versions,
			CommandVersion{Revision: revision, Values: cloneValues(h.versions[revision])},
		)
	}
	if pending != nil {
		snapshot.Revision = pending.Revision
		snapshot.Versions = append(
			snapshot.Versions,
			CommandVersion{Revision: pending.Revision, Values: cloneValues(pending.Values)},
		)
	}
	for key, revision := range h.boundaries {
		snapshot.Boundaries = append(
			snapshot.Boundaries,
			SessionBoundary{BlockID: key.block, Edge: key.edge, CommandRevision: revision},
		)
	}
	rank := map[BoundaryEdge]int{BeforeUser: 0, AfterAssistant: 1, AfterBlock: 2}
	slices.SortFunc(snapshot.Boundaries, func(a, b SessionBoundary) int {
		if order := strings.Compare(string(a.BlockID), string(b.BlockID)); order != 0 {
			return order
		}
		return rank[a.Edge] - rank[b.Edge]
	})
	return snapshot
}

// TakeUpdate returns changed state, or state carrying pending, and clears dirtiness.
func (h *CommandStateHistory) TakeUpdate(pending *CommandVersion) *CommandStateSnapshot {
	if !h.dirty && pending == nil {
		return nil
	}
	h.dirty = false
	return new(h.Snapshot(pending))
}

// MarkDirty makes a failed save's next attempt carry command state again.
func (h *CommandStateHistory) MarkDirty() { h.dirty = true }

// cloneValues gives a command version its own raw JSON values.
func cloneValues(values map[CommandStorageKey]json.RawMessage) map[CommandStorageKey]json.RawMessage {
	copied := make(map[CommandStorageKey]json.RawMessage, len(values))
	for key, value := range values {
		copied[key] = bytes.Clone(value)
	}
	return copied
}

// canonicalValues compares complete command versions using RFC 8785.
func canonicalValues(values map[CommandStorageKey]json.RawMessage) ([]byte, error) {
	data, err := contract.EncodeJSON(values)
	if err != nil {
		return nil, err
	}
	return jcs.Transform(data)
}
