package database

import (
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// WakeupDue identifies when a saved yield wakeup is due.
//
//sumtype:decl
type WakeupDue interface{ wakeupDue() }

// WakeupAtStart is due at the next start because its action had not ended.
type WakeupAtStart struct{}

func (*WakeupAtStart) wakeupDue() {}

// WakeupAt is due at its saved timestamp.
type WakeupAt struct{ At core.Timestamp }

func (*WakeupAt) wakeupDue() {}

// Saved reports each checkpoint or deletion commit and the tree's earliest saved wakeup.
// A nil due means no saved wakeup. Notification happens after the transaction ends.
type Saved func(node core.NodeID, due WakeupDue)

// Terminal identifies how the root's latest finished request ended.
type Terminal uint8

const (
	// TerminalResponse is a completed response.
	TerminalResponse Terminal = iota
	// TerminalError is a failed request.
	TerminalError
	// TerminalAbort is an aborted request.
	TerminalAbort
)

// EmptySummary returns the facts before a tree exists: idle, revision zero.
func EmptySummary() SummaryFacts {
	return SummaryFacts{Phase: core.SessionPhaseIdle}
}

// NodeHistory is a subagent's record and blocks in a cold history read.
type NodeHistory struct {
	Record store.NodeRecord
	Blocks []core.Block
}
