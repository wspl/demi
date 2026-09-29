package provider

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"iter"
	"net/http"

	"github.com/wspl/demi/go/core"
)

// Provider is one entry and account, shared across sessions.
type Provider interface {
	Capabilities() Capabilities
	AuthStatus(context.Context) core.AuthState
	RuntimeState() core.RuntimeState
	ListModels(context.Context) (core.ProviderModelList, error)
	ReadFailure(core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts
	Runtime(RuntimeEnv) (ProviderRuntime, error)
}
type QuotaProvider interface{ Quota() *ProviderQuota }
type AccountsProvider interface{ Accounts() SubscriptionAccounts }
type Capabilities struct{ ProcessHost bool }
type RuntimeEnv struct{ HTTP *http.Client }
type ProcessHostRequired struct{ Provider string }

func (e ProcessHostRequired) Error() string {
	return fmt.Sprintf("%s needs a Host that runs processes", e.Provider)
}

type CatalogErrorKind string

const (
	CatalogUnauthenticated CatalogErrorKind = "unauthenticated"
	CatalogUnavailable     CatalogErrorKind = "unavailable"
	CatalogInvalid         CatalogErrorKind = "invalid"
)

type CatalogError struct {
	Kind    CatalogErrorKind
	Message string
}

func (e *CatalogError) Error() string { return e.Message }

// ProviderRuntime belongs to one session and runs one request at a time.
// The iterator ends on cancellation, a response, a failure, or a tool batch.
// Stopping iteration releases resources belonging to the run.
type ProviderRuntime interface {
	Run(context.Context, InferenceRequest) iter.Seq[ProviderEvent]
	Fresh() ProviderRuntime
	Close(context.Context) error
	RequestLimits(core.Model) RequestLimits
}
type RequestLimits struct {
	BodyBytes *uint64
	Images    *uint32
}

func OpenAIRequestLimits() RequestLimits {
	body, images := uint64(512000000), uint32(1500)
	return RequestLimits{BodyBytes: &body, Images: &images}
}
func AnthropicRequestLimits(model core.Model) RequestLimits {
	body, images := uint64(32000000), uint32(100)
	if model.ContextWindow > 200000 {
		images = 600
	}
	return RequestLimits{BodyBytes: &body, Images: &images}
}

type InferenceRequest struct {
	SessionID, TurnID, RequestID, ModelID string
	OutputLimit, OutputCap                *uint32
	SystemPrompt                          string
	Items                                 []InferenceItem
	Tools                                 []ToolDefinition
	Thinking                              core.ThinkingConfig
	ServiceTierID                         *string
	// Nil disables caching; a value requests session caching.
	PromptCache *PromptCache
}
type PromptCache struct{ AnsweredItems int }

func (r InferenceRequest) MaxOutputTokens() *uint32 {
	if r.OutputLimit == nil {
		return r.OutputCap
	}
	if r.OutputCap == nil || *r.OutputLimit < *r.OutputCap {
		return r.OutputLimit
	}
	return r.OutputCap
}

type InferenceItem interface{ inferenceItem() }
type UserMessage struct{ Content []UserPart }
type UserSteer struct{ Content []UserPart }
type AssistantText struct{ ModelID, Text string }
type AssistantThinking struct {
	ModelID, Text   string
	Signature       *string
	KeptPastSummary bool
}
type AssistantRedactedThinking struct {
	ModelID, Data   string
	KeptPastSummary bool
}
type ToolUse struct {
	ModelID, ToolUseID, ToolName string
	Input                        jsontext.Value
}
type ToolResult struct {
	ToolUseID string
	Output    []ResultPart
	IsError   bool
}

func (UserMessage) inferenceItem()               {}
func (UserSteer) inferenceItem()                 {}
func (AssistantText) inferenceItem()             {}
func (AssistantThinking) inferenceItem()         {}
func (AssistantRedactedThinking) inferenceItem() {}
func (ToolUse) inferenceItem()                   {}
func (ToolResult) inferenceItem()                {}

type UserPart interface{ userPart() }
type TextPart struct{ Text string }
type ImagePart struct{ Medium Medium }
type VideoPart struct{ Medium Medium }
type DocumentPart struct {
	Bytes    MediaBytes
	FileName string
}

func (TextPart) userPart()     {}
func (ImagePart) userPart()    {}
func (VideoPart) userPart()    {}
func (DocumentPart) userPart() {}

type Medium interface{ medium() }
type MediaBytes struct {
	Data      core.B64Bytes
	MediaType string
}
type MediaURL struct{ URL string }

func (MediaBytes) medium() {}
func (MediaURL) medium()   {}

type ResultPart interface{ resultPart() }
type ResultImage struct{ Bytes MediaBytes }
type ResultVideo struct{ Bytes MediaBytes }

func (TextPart) resultPart()    {}
func (ResultImage) resultPart() {}
func (ResultVideo) resultPart() {}

type ToolDefinition struct {
	Name, Description string
	InputSchema       jsontext.Value
}
type ProviderEvent interface{ providerEvent() }
type ThinkingStart struct{}
type ThinkingDelta struct{ Text string }
type ThinkingSignature struct{ Signature string }
type RedactedThinking struct{ Data string }
type TextDelta struct{ Text string }
type ToolCall struct {
	ToolUseID, ToolName string
	Input               jsontext.Value
}
type Response struct{ Usage core.TokenUsage }
type FailureEvent struct{ Failure ProviderFailure }

func (ThinkingStart) providerEvent()     {}
func (ThinkingDelta) providerEvent()     {}
func (ThinkingSignature) providerEvent() {}
func (RedactedThinking) providerEvent()  {}
func (TextDelta) providerEvent()         {}
func (ToolCall) providerEvent()          {}
func (Response) providerEvent()          {}
func (FailureEvent) providerEvent()      {}
