package store

// Contract comments are product text: contractgen emits them as schema descriptions.
//revive:disable:exported

import (
	"encoding/json"

	"github.com/wspl/demi/internal/core"
)

// A key of a node's command storage: a nonempty relative name with no NUL,
// no leading `/` or drive letter, and no `..` segment.
// +demi:root
// +demi:id
// +demi:check validateCommandStorageKey
type CommandStorageKey string

// Where a boundary sits relative to its block.
// +demi:root
// +demi:enum before_user after_assistant after_block
type BoundaryEdge string

const (
	// Before a block that opens an input turn: the version current when the
	// session started processing the turn. Recorded once.
	BeforeUser BoundaryEdge = "before_user"
	// When assistant text completed. Recorded once.
	AfterAssistant BoundaryEdge = "after_assistant"
	// When the block last changed; it moves forward with the block.
	AfterBlock BoundaryEdge = "after_block"
)

// One immutable version: the complete map of the node's keys.
// +demi:root
type CommandVersion struct {
	Revision uint64                                `json:"revision"`
	Values   map[CommandStorageKey]json.RawMessage `json:"values"`
}

// The version that was current at one edge of one block.
// +demi:root
type SessionBoundary struct {
	BlockID         core.BlockID `json:"blockId"`
	Edge            BoundaryEdge `json:"edge"`
	CommandRevision uint64       `json:"commandRevision"`
}

// A node's command state as its checkpoint carries it.
// +demi:root
type CommandStateSnapshot struct {
	// The current version.
	Revision   uint64            `json:"revision"`
	Versions   []CommandVersion  `json:"versions"`
	Boundaries []SessionBoundary `json:"boundaries"`
}
