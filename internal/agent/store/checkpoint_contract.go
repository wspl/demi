package store

//go:generate go run github.com/wspl/demi/tools/contractgen

// Contract comments are product text copied verbatim from Rust.
//revive:disable:exported

import "github.com/wspl/demi/internal/core"

// The state row of a node's checkpoint: everything the session saves
// beside its transcript rows and command state.
// +demi:root
type CheckpointState struct {
	// `running` in a checkpoint the process died in, or that dispose wrote
	// under a running turn.
	Phase core.SessionPhase `json:"phase"`
	// The queued messages, in the order they run.
	Queue []core.QueuedMessage `json:"queue"`
	// The agent messages waiting for a continuation boundary, in admission
	// order.
	AgentInputs []PendingAgentInput `json:"agentInputs"`
	// The yield wakeups not yet written into the transcript, fired or not.
	Wakeups []ScheduledWakeup `json:"wakeups"`
	// +demi:length chars min=1
	CWD   string              `json:"cwd"`
	Model core.ModelSelection `json:"model"`
	// The receipts of the accepted edits.
	Edits []EditReceipt `json:"edits"`
}

// An agent message the session admitted and has not yet written into its
// transcript (`subagents.md` § Durable ownership and replay). Its id and
// body live only in the message.
// +demi:root
type PendingAgentInput struct {
	// The turn it was admitted for; a continuation writes it into its own.
	TurnID core.TurnID `json:"turnId"`
	// The model selection current at admission, which its block records.
	Model   core.ModelSelection `json:"model"`
	Message core.AgentMessage   `json:"message"`
}

// A yield wakeup (`runtime.md` § Yield wakeups).
// +demi:root
type ScheduledWakeup struct {
	ID core.WakeupID `json:"id"`
	// How long after the scheduling action ended it fires.
	// +demi:range min=1
	DurationMS uint32 `json:"durationMs"`
	// When it is due, in wall-clock time; null until the action that
	// scheduled it ended.
	// +demi:nullable
	DueAt *core.Timestamp `json:"dueAt"`
}

// The receipt of an accepted edit (`message-editing.md` § Commit and
// idempotency).
// +demi:root
type EditReceipt struct {
	OperationID core.OperationID `json:"operationId"`
	// The SHA-256 of the request's RFC 8785 canonical JSON, as the web app
	// sent it, in lowercase hexadecimal.
	// +demi:pattern ^[0-9a-f]{64}$
	Digest string `json:"digest"`
	// The replacement's turn.
	TurnID core.TurnID `json:"turnId"`
}
