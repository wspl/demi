// Package agentdata declares the Rust agent's persisted checkpoint encoding.
// G7d should reuse these records, or move their owner without changing the bytes.
package agentdata

//go:generate go run ../../../cmd/wiregen

import (
	"regexp"

	"github.com/wspl/demi/go/core"
)

var DigestPattern = regexp.MustCompile(`^[0-9a-f]{64}$`)

//demi:wire
type CheckpointState struct {
	Phase       core.SessionPhase    `json:"phase" check:"func=core.ValidateSessionPhase"`
	Queue       []core.QueuedMessage `json:"queue" check:"each(func=core.Validate)"`
	AgentInputs []PendingAgentInput  `json:"agentInputs"`
	Wakeups     []ScheduledWakeup    `json:"wakeups"`
	Cwd         string               `json:"cwd" check:"chars=1.."`
	Model       core.ModelSelection  `json:"model" check:"func=core.Validate"`
	Harness     string               `json:"harness" check:"chars=1.."`
	Edits       []EditReceipt        `json:"edits"`
}

//demi:wire
type PendingAgentInput struct {
	TurnID  core.TurnID         `json:"turnId" check:"func=core.Validate"`
	Model   core.ModelSelection `json:"model" check:"func=core.Validate"`
	Message core.AgentMessage   `json:"message" check:"func=core.Validate"`
}

//demi:wire
type ScheduledWakeup struct {
	ID         core.WakeupID   `json:"id" check:"func=core.Validate"`
	DurationMs uint32          `json:"durationMs" check:"range=1.."`
	DueAt      *core.Timestamp `json:"dueAt" check:"nullable,func=core.Validate"`
}

//demi:wire
type EditReceipt struct {
	OperationID core.OperationID `json:"operationId" check:"func=core.Validate"`
	Digest      string           `json:"digest" check:"pattern=DigestPattern"`
	TurnID      core.TurnID      `json:"turnId" check:"func=core.Validate"`
}
