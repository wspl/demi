package server

import (
	"github.com/wspl/demi/internal/core"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// The input of `demi agent spawn`.
// +demi:root
// +demi:schema
type spawnArgs struct {
	Prompt string `json:"prompt"`
	// Stable id for this creation or resume request. Supply the same id and
	// arguments to retry safely after an uncertain response; otherwise a new
	// id is generated.
	// +demi:length chars min=1 max=128
	RequestID *string `json:"request-id,omitempty"`
	Profile   *string `json:"profile,omitempty"`
	// Short UI title distinguishing concurrent children.
	Description *string `json:"description,omitempty"`
	// Forbid this child from spawning subagents of its own; it can still
	// send, list, and show.
	NoSubagents *bool `json:"no-subagents,omitempty"`
}

// The input of `demi agent send`.
// +demi:root
// +demi:schema
type sendArgs struct {
	// Target agent number from the tree, or "parent" for the session that
	// spawned this one
	ID string `json:"id"`
	// Message body.
	Message string `json:"message"`
}

// The input of `demi agent abort`.
// +demi:root
// +demi:schema
type abortArgs struct {
	// subagentId from spawn stdout
	ID uint64 `json:"id"`
}

// The input of `demi agent resume`.
// +demi:root
// +demi:schema
type resumeArgs struct {
	// subagentId of an archived child
	ID uint64 `json:"id"`
	// Stable id for this creation or resume request. Supply the same id and
	// arguments to retry safely after an uncertain response; otherwise a new
	// id is generated.
	// +demi:length chars min=1 max=128
	RequestID *string `json:"request-id,omitempty"`
	// The reviving user message.
	Message string `json:"message"`
}

// The input of `demi agent list`, which takes none.
// +demi:root
// +demi:schema
type listArgs struct{}

// The input of `demi agent show`.
// +demi:root
// +demi:schema
type showArgs struct {
	// Agent number from the tree
	ID uint64 `json:"id"`
}

// `demi agent spawn --json` and `resume --json`.
// +demi:root
// +demi:schema
type started struct {
	SubagentID uint64 `json:"subagentId"`
}

// `demi agent send --json`.
// +demi:root
// +demi:schema
type sent struct {
	ID       uint64 `json:"id"`
	Accepted bool   `json:"accepted"`
}

// `demi agent abort --json`.
// +demi:root
// +demi:schema
type aborted struct {
	ID      uint64 `json:"id"`
	Aborted bool   `json:"aborted"`
}

// What a start does, as its reservation records it.
// +demi:union tag=kind
//
//sumtype:decl
type startInput interface{ startInput() }

// +demi:variant startInput spawn
type spawnInput struct {
	Prompt string `json:"prompt"`
	// +demi:nullable
	ProfileName      *string `json:"profileName"`
	Description      string  `json:"description"`
	IsSpawnForbidden bool    `json:"isSpawnForbidden"`
}

// +demi:variant startInput resume
type resumeInput struct {
	ID      core.NodeID `json:"id"`
	Message string      `json:"message"`
}

// A start's immutable reservation in the owner's command storage, at
// `agent.start.<request id>` (`subagents.md` § Model-facing surface).
// +demi:root
type startReceipt struct {
	Input  startInput  `json:"input"`
	NodeID core.NodeID `json:"nodeId"`
	// The round the start begins: 1 for a spawn, one more than the child's
	// last for a resume.
	Round uint64 `json:"round"`
}

// The input of `demi shell output`.
// +demi:root
// +demi:schema
type outputArgs struct {
	// The command's commandId, as its result names it
	ID string `json:"id"`
	// The lines to print, as <from>-<to>
	// +demi:pattern ^[0-9]+-[0-9]+$
	Lines *string `json:"lines,omitempty"`
	// Print the last n lines
	// +demi:range min=1
	Tail *uint64 `json:"tail,omitempty"`
	// Only stdout
	Stdout *bool `json:"stdout,omitempty"`
	// Only stderr
	Stderr *bool `json:"stderr,omitempty"`
	// The bytes as they are: unnumbered and unpaged
	Raw *bool `json:"raw,omitempty"`
}

func (*spawnInput) startInput()  {}
func (*resumeInput) startInput() {}
