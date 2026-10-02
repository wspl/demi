package database

import (
	"github.com/wspl/demi/internal/core"
)

// SummaryFacts describes what a conversation's summary is built from (`storage.md` § Conversation
// state and transactions): the root's phase and output revision and the
// kind of its latest terminal block, read without loading the transcript.
type SummaryFacts struct {
	Phase    core.SessionPhase
	Revision uint64
	Last     *Terminal
}

// History is a tree's history as its database holds it: the root's blocks, and each
// subagent's record with its blocks, depth first in spawn order.
type History struct {
	Blocks    []core.Block
	Subagents []NodeHistory
}
