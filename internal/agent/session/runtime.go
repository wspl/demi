package session

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider"
)

// Runtime supplies a session's admission, prompts, context and tools. The node
// assembly implements it; tools return outcomes without reaching into a session.
// Calls run outside the session's state lock and must honor cancellation.
type Runtime interface {
	// EnterAction waits for the tree's admission and transfers the lease to
	// the session, which releases it after the action ends.
	EnterAction(ctx context.Context) (*gates.Lease, error)
	// ReserveEdit holds off changes of the node's children until the session
	// releases the reservation. Nil means no reservation is needed. Live,
	// starting or closing children and undelivered completions refuse it.
	ReserveEdit(ctx context.Context) (*gates.Reservation, error)
	// SystemPrompt returns the assembled node's system prompt.
	SystemPrompt(ctx context.Context) (string, error)
	// Preamble returns the text before a user turn's content, or nil.
	Preamble(ctx context.Context) (*string, error)
	// Context returns new context in source order. Seen contains replayed
	// context from the last compaction boundary, oldest first.
	Context(ctx context.Context, seen []SeenContext, turn core.TurnID) ([]NewContext, error)
	// Tools returns immutable definitions of the tools the model may call.
	Tools() []provider.ToolDefinition
	// InvokeTool runs one named call. Its context has the action's lifetime:
	// commands started by the call remain bound to it after this method returns.
	// A tool failure returns *ToolFailure; cancellation returns ctx.Err().
	InvokeTool(ctx context.Context, call ToolInvocation) (ToolOutcome, error)
	// Dispose releases the tools' environments and commands. It joins owned
	// work before returning; the session awaits it during disposal.
	Dispose(ctx context.Context) error
}

// SeenContext is a context block the model receives, as its source sees it.
type SeenContext struct {
	Source string
	Text   string
}

// NewContext is what one context source answered: a new block's source and text.
type NewContext struct {
	Source string
	Text   string
}

// ToolInvocation is one call of a tool. Cancellation is InvokeTool's first
// argument, and belongs to the action rather than just the invocation.
type ToolInvocation struct {
	// ToolUseID is the provider's call id, named by pages showing its command.
	ToolUseID string
	ToolName  string
	// Input preserves the provider's JSON value and object order, or is a
	// JSON string containing invalid JSON input. The tool validates it.
	Input json.RawMessage
	// Model is the model that asked for the call; its accepted media govern
	// the result's attachments.
	Model core.ModelSelection
	// RequestLimits bounds the video a result may attach.
	RequestLimits provider.RequestLimits
	// Generation is the node's command-storage generation a new job records.
	Generation uint64
}

// ToolOutcome is how a tool call completed.
type ToolOutcome struct {
	// Output holds the result and media bytes, stored before transcript entry.
	Output  []provider.ResultPart
	IsError bool
	View    core.ToolView
	// Effect asks the session to act beyond recording output; nil means none.
	Effect ToolEffect
}

// ErrorOutcome returns a call that completed as an error with this text.
func ErrorOutcome(text string) ToolOutcome { panic("not written: a-session") }

// ToolEffect is what a tool asks of its session. A tool never reaches into it.
//
//sumtype:decl
type ToolEffect interface{ toolEffect() }

// ScheduleYield schedules a wakeup after the action ends and ends the turn
// after this round unless input arrived during it. The session writes the
// result text and view of this effect itself.
type ScheduleYield struct{ DurationMS uint32 }

func (*ScheduleYield) toolEffect() {}

// ToolFailure is a tool that failed; its call completes as Tool failed: <message>.
type ToolFailure struct{ Message string }

// Error returns the tool failure's message.
func (e *ToolFailure) Error() string { panic("not written: a-session") }
