package session

//go:generate go run github.com/wspl/demi/tools/contractgen

// Contract comments are product text copied verbatim from Rust.
//revive:disable:exported

// What a session is doing, as a supervisor observes it (`subagents.md`
// § `demi agent show`); not the phase that `phase` frames carry.
// +demi:root
// +demi:schema
// +demi:enum idle provider_streaming tool_executing compacting finalizing pending_yield
type Execution string

const (
	// Idle means the session has no running action or yield wakeup.
	Idle Execution = "idle"
	// ProviderStreaming means a provider request is running.
	ProviderStreaming Execution = "provider_streaming"
	// ToolExecuting means a tool call is running.
	ToolExecuting Execution = "tool_executing"
	// Compacting means a compaction pass is running.
	Compacting Execution = "compacting"
	// The action ended and its checkpoint is being saved.
	Finalizing Execution = "finalizing"
	// Idle with a yield wakeup scheduled.
	PendingYield Execution = "pending_yield"
)
