package transcript

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"encoding/json"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// ReplayChars is the longest text replay sends unchanged, in Unicode scalars.
const ReplayChars = 16_000

// ResumeText is what the model receives for a resume block.
const ResumeText = "Continue from where you left off."

// WakeupText is what the model receives for a fired yield wakeup.
const WakeupText = "Scheduled yield wakeup fired. Continue the previous work and inspect any running command with shell_status when needed."

// Replayed is what a request carries of a transcript.
type Replayed struct {
	// Items contains the blocks' inference items in order.
	Items []provider.InferenceItem
	// Answered counts leading items carried by the latest answered request.
	Answered int
}

// RequestView owns the interpretation of media for one model's request: held
// bytes within accepted types and half the body limit, otherwise stable text.
// Construct it with NewRequestView and treat its input view as immutable.
type RequestView struct{}

// NewRequestView selects how model receives view within its vendor's limits.
func NewRequestView(view *store.ModelView, model core.Model, limits provider.RequestLimits) *RequestView {
	panic("not written: a-transcript")
}

// Model returns the request's model.
func (r *RequestView) Model() core.Model { panic("not written: a-transcript") }

// Replay renders blocks from the latest compaction boundary in order, preserving
// signed reasoning and opaque data whole and marking reasoning kept past a summary.
func Replay(request *RequestView) Replayed { panic("not written: a-transcript") }

// ToolInput returns provider-supplied JSON, or a JSON string when input is invalid.
// Objects retain their read order, including nested objects.
func ToolInput(input string) json.RawMessage { panic("not written: a-transcript") }

// AgentMessageEnvelope renders an agent message as its model-facing instruction
// and JSON envelope, naming its sender by number and round, without delivery ids.
func AgentMessageEnvelope(message core.AgentMessage) string { panic("not written: a-transcript") }
