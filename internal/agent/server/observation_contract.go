package server

import (
	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/conversationproto"
)

// +demi:enum root live archived
type entryKind string

// +demi:enum executing completed error
type toolStatus string

// `demi agent list --json`.
// +demi:root
// +demi:schema
type listing struct {
	Tree []treeEntry `json:"tree"`
}

// `demi agent show --json`.
// +demi:root
// +demi:schema
type shown struct {
	Agent agentSnapshot `json:"agent"`
}

// One node of `demi agent list --json`.
type treeEntry struct {
	SubagentID uint64 `json:"subagentId"`
	// +demi:nullable
	ParentSessionID *uint64   `json:"parentSessionId"`
	Kind            entryKind `json:"kind"`
	Description     string    `json:"description"`
	// +demi:nullable
	Profile *string                    `json:"profile"`
	Phase   conversationproto.JobPhase `json:"phase"`
	// +demi:nullable
	ClosedAgoMS *uint64 `json:"closedAgoMs"`
	Self        bool    `json:"self"`
}

// `demi agent show --json`'s agent: every duration in milliseconds before
// the query.
type agentSnapshot struct {
	SubagentID      uint64 `json:"subagentId"`
	ParentSessionID uint64 `json:"parentSessionId"`
	Description     string `json:"description"`
	// +demi:nullable
	Profile     *string                    `json:"profile"`
	Phase       conversationproto.JobPhase `json:"phase"`
	ElapsedMS   uint64                     `json:"elapsedMs"`
	LastEventMS uint64                     `json:"lastEventMs"`
	Execution   session.Execution          `json:"execution"`
	Activity    string                     `json:"activity"`
	// How long the current execution state has lasted.
	ExecutionForMS    uint64         `json:"executionForMs"`
	Tools             []toolSnapshot `json:"tools"`
	LastAssistantText string         `json:"lastAssistantText"`
	// +demi:nullable
	LastAssistantTextAgoMS *uint64 `json:"lastAssistantTextAgoMs"`
}

type toolSnapshot struct {
	Title      string     `json:"title"`
	Status     toolStatus `json:"status"`
	DurationMS uint64     `json:"durationMs"`
	// +demi:nullable
	EndedAgoMS *uint64 `json:"endedAgoMs"`
}

// +demi:root
// +demi:tolerant
type titledCall struct {
	Description string `json:"description"`
}
