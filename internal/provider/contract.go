package provider

import (
	"context"
	"encoding/json"
	"iter"
	"net/http"

	"github.com/wspl/demi/internal/core"
)

// Provider is one provider entry and account, shared by every user and request.
type Provider interface {
	Capabilities() Capabilities
	AuthStatus(context.Context) core.AuthState
	RuntimeState() core.RuntimeState
	ListModels(context.Context) (core.ProviderModelList, error)
	ReadFailure(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts
	Quota() *Quota
	Accounts() SubscriptionAccounts
	Runtime(RuntimeEnv) (Runtime, error)
}

// Capabilities describes what a provider requires of its conversation's Host.
type Capabilities struct{ ProcessHost bool }

// RuntimeEnv supplies the HTTP client of the runtime's owner.
type RuntimeEnv struct{ HTTP *http.Client }

// Runtime runs one session's requests, one at a time. Close releases retained resources.
type Runtime interface {
	Run(context.Context, InferenceRequest) Run
	Fresh() Runtime
	Close(context.Context) error
	RequestLimits(core.Model) RequestLimits
}

// Run yields the events of one inference attempt. The consumer must stop the
// iterator on cancellation; ending iteration releases the attempt's resources.
type Run = iter.Seq[Event]

// RuntimeError explains why a runtime could not be built.
type RuntimeError struct{ Provider string }

// Error returns the diagnostic for this failure.
func (e *RuntimeError) Error() string {
	return e.Provider + " needs a Host that runs processes"
}

// CatalogError explains why the model directory could not be read.
type CatalogError struct {
	Kind    CatalogErrorKind
	Message string
}

// Error returns the diagnostic for this failure.
func (e *CatalogError) Error() string { return e.Message }

// CatalogErrorKind distinguishes authentication, availability and invalid input.
type CatalogErrorKind uint8

// Catalog failure categories exposed to callers.
const (
	// CatalogUnauthenticated indicates that authentication prevented catalog retrieval.
	CatalogUnauthenticated CatalogErrorKind = iota
	// CatalogUnavailable indicates that the catalog could not be retrieved.
	CatalogUnavailable
	// CatalogInvalid indicates that the catalog answer could not be decoded.
	CatalogInvalid
)

// RequestLimits describes what a vendor accepts in one request; nil means undocumented.
type RequestLimits struct {
	BodyBytes *uint64
	Images    *uint32
}

// OpenAIRequestLimits returns the OpenAI API limits, also used by Codex.
func OpenAIRequestLimits() RequestLimits {
	body, images := uint64(512_000_000), uint32(1500)
	return RequestLimits{BodyBytes: &body, Images: &images}
}

// AnthropicRequestLimits returns the Messages API limits for model.
func AnthropicRequestLimits(model core.Model) RequestLimits {
	body, images := uint64(32_000_000), uint32(100)
	if model.ContextWindow > 200_000 {
		images = 600
	}
	return RequestLimits{BodyBytes: &body, Images: &images}
}

// InferenceRequest is one inference request of a session. Run's context owns cancellation.
type InferenceRequest struct {
	SessionID     string
	TurnID        string
	RequestID     string
	ModelID       string
	OutputLimit   *uint32
	OutputCap     *uint32
	SystemPrompt  string
	Items         []InferenceItem
	Tools         []ToolDefinition
	Thinking      core.ThinkingConfig
	ServiceTierID *string
	PromptCache   PromptCache
}

// MaxOutputTokens returns the model limit lowered to the request cap, or either alone.
func (r InferenceRequest) MaxOutputTokens() *uint32 {
	if r.OutputLimit == nil {
		return r.OutputCap
	}
	if r.OutputCap == nil || *r.OutputLimit < *r.OutputCap {
		return r.OutputLimit
	}
	return r.OutputCap
}

// PromptCache names the answered prefix of a session; nil AnsweredItems disables caching.
type PromptCache struct{ AnsweredItems *int }

// InferenceItem is one entry of the transcript as a provider replays it.
//
//sumtype:decl
type InferenceItem interface{ inferenceItem() }

// UserMessage carries the user's message.
type UserMessage struct{ Content []UserPart }

// UserSteer carries a steer or agent message that joined the running turn.
type UserSteer struct{ Content []UserPart }

// AssistantText carries a model's text.
type AssistantText struct {
	ModelID string
	Text    string
}

// AssistantThinking carries reasoning and its optional vendor signature.
type AssistantThinking struct {
	ModelID         string
	Text            string
	Signature       *string
	KeptPastSummary bool
}

// AssistantRedactedThinking carries opaque reasoning as received.
type AssistantRedactedThinking struct {
	ModelID         string
	Data            string
	KeptPastSummary bool
}

// ToolUse carries the model's tool call, including invalid JSON as a JSON string.
type ToolUse struct {
	ModelID   string
	ToolUseID string
	ToolName  string
	Input     json.RawMessage
}

// ToolResult carries the tool's output.
type ToolResult struct {
	ToolUseID string
	Output    []ResultPart
	IsError   bool
}

func (*UserMessage) inferenceItem()               {}
func (*UserSteer) inferenceItem()                 {}
func (*AssistantText) inferenceItem()             {}
func (*AssistantThinking) inferenceItem()         {}
func (*AssistantRedactedThinking) inferenceItem() {}
func (*ToolUse) inferenceItem()                   {}
func (*ToolResult) inferenceItem()                {}

// UserPart is one part of a message or steer, with resolved media.
//
//sumtype:decl
type UserPart interface{ userPart() }

// TextPart carries text in a user message or tool result.
type TextPart struct{ Text string }

// ImagePart carries an image in a user message.
type ImagePart struct{ Medium Medium }

// VideoPart carries a video in a user message.
type VideoPart struct{ Medium Medium }

// DocumentPart carries a PDF and its original file name.
type DocumentPart struct {
	Bytes    MediaBytes
	FileName string
}

func (*TextPart) userPart()     {}
func (*ImagePart) userPart()    {}
func (*VideoPart) userPart()    {}
func (*DocumentPart) userPart() {}

// Medium is an image or video's bytes or its vendor-fetchable URL.
//
//sumtype:decl
type Medium interface{ medium() }

// MediaBytes holds a medium's bytes and media type.
type MediaBytes struct {
	Data      []byte
	MediaType string
}

// MediaURL is a URL the vendor fetches.
type MediaURL struct{ URL string }

func (*MediaBytes) medium() {}
func (*MediaURL) medium()   {}

// ResultPart is text, or an image or video with its bytes.
//
//sumtype:decl
type ResultPart interface{ resultPart() }

// ResultImage carries a tool's image bytes.
type ResultImage struct{ Bytes MediaBytes }

// ResultVideo carries a tool's video bytes.
type ResultVideo struct{ Bytes MediaBytes }

func (*TextPart) resultPart()    {}
func (*ResultImage) resultPart() {}
func (*ResultVideo) resultPart() {}

// ToolDefinition describes a tool the model may call.
type ToolDefinition struct {
	Name        string
	Description string
	InputSchema json.RawMessage
}

// Event is an event of a run. Response and Error end a run; cancellation ends
// it without a further event. All inference failures are reported as Error.
//
//sumtype:decl
type Event interface{ event() }

// ThinkingStart opens a reasoning block.
type ThinkingStart struct{}

// ThinkingDelta extends the reasoning text.
type ThinkingDelta struct{ Text string }

// ThinkingSignature supplies the vendor signature for replay.
type ThinkingSignature struct{ Signature string }

// RedactedThinking carries encrypted reasoning as received.
type RedactedThinking struct{ Data string }

// TextDelta extends the answer text.
type TextDelta struct{ Text string }

// ToolCall is a tool call the model requested.
type ToolCall struct {
	ToolUseID string
	ToolName  string
	Input     json.RawMessage
}

// Response carries the usage of the run's final API call, not a turn total.
type Response struct{ Usage core.TokenUsage }

// Error is the run's terminal failure.
type Error struct{ Failure Failure }

func (*ThinkingStart) event()     {}
func (*ThinkingDelta) event()     {}
func (*ThinkingSignature) event() {}
func (*RedactedThinking) event()  {}
func (*TextDelta) event()         {}
func (*ToolCall) event()          {}
func (*Response) event()          {}
func (*Error) event()             {}

// VendorPolicy specifies a vendor's request requirements.
type VendorPolicy struct {
	PassBackReasoningContent bool
	ReplayAssistantStatus    bool
	EffortAsBudget           bool
}

// EffortBudget maps an effort to its token budget; unknown efforts use medium.
func EffortBudget(effort string) uint32 {
	switch effort {
	case "low":
		return 4096
	case "high":
		return 32768
	case "xhigh":
		return 65536
	case "max":
		return 98304
	default:
		return 16384
	}
}
