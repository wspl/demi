package providertest

import (
	"context"
	"encoding/base64"
	"encoding/json/jsontext"
	"fmt"
	"iter"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

type FixedClock struct{ Time core.Timestamp }

func (c FixedClock) Now() core.Timestamp { return c.Time }
func InferenceRequest() provider.InferenceRequest {
	return provider.InferenceRequest{SessionID: "session-1", TurnID: "turn-1", RequestID: "request-1", ModelID: "model-1", Items: []provider.InferenceItem{provider.UserMessage{Content: []provider.UserPart{provider.TextPart{Text: "hello"}}}}, Tools: []provider.ToolDefinition{}}
}

type Turn func(context.Context, provider.InferenceRequest) iter.Seq[provider.ProviderEvent]

func Events(events ...provider.ProviderEvent) Turn {
	return func(context.Context, provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
		return slices.Values(events)
	}
}
func Respond(respond func(provider.InferenceRequest) []provider.ProviderEvent) Turn {
	return func(_ context.Context, request provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
		return slices.Values(respond(request))
	}
}
func Pending() Turn {
	return func(ctx context.Context, _ provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
		return func(func(provider.ProviderEvent) bool) { <-ctx.Done() }
	}
}

type script struct {
	mu       sync.Mutex
	turns    []Turn
	requests []provider.InferenceRequest
	closes   int
	limits   provider.RequestLimits
}

// ScriptedRuntime and its fresh copies consume one ordered scenario.
type ScriptedRuntime struct{ script *script }

func NewScriptedRuntime(turns ...Turn) *ScriptedRuntime {
	return &ScriptedRuntime{script: &script{turns: slices.Clone(turns)}}
}
func (r *ScriptedRuntime) WithLimits(limits provider.RequestLimits) *ScriptedRuntime {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	r.script.limits = limits
	return r
}
func (r *ScriptedRuntime) Requests() []provider.InferenceRequest {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return slices.Clone(r.script.requests)
}
func (r *ScriptedRuntime) Remaining() int {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return len(r.script.turns)
}
func (r *ScriptedRuntime) Closes() int {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return r.script.closes
}
func (r *ScriptedRuntime) Run(ctx context.Context, request provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
	return func(yield func(provider.ProviderEvent) bool) {
		r.script.mu.Lock()
		r.script.requests = append(r.script.requests, request)
		if len(r.script.turns) == 0 {
			number := len(r.script.requests)
			r.script.mu.Unlock()
			panic(fmt.Sprintf("ScriptedRuntime: no turn scripted for run #%d", number))
		}
		turn := r.script.turns[0]
		r.script.turns = r.script.turns[1:]
		r.script.mu.Unlock()
		if ctx.Err() != nil {
			return
		}
		for event := range turn(ctx, request) {
			if ctx.Err() != nil || !yield(event) {
				return
			}
		}
	}
}
func (r *ScriptedRuntime) Fresh() provider.ProviderRuntime { return &ScriptedRuntime{script: r.script} }
func (r *ScriptedRuntime) Close(context.Context) error {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	r.script.closes++
	return nil
}
func (r *ScriptedRuntime) RequestLimits(core.Model) provider.RequestLimits {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return r.script.limits
}

// Run bounds a scenario by context, so a missing terminal event fails its test.
func Run(t *testing.T, runtime provider.ProviderRuntime, request provider.InferenceRequest) []provider.ProviderEvent {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	events := slices.Collect(runtime.Run(ctx, request))
	if err := ctx.Err(); err != nil {
		t.Fatalf("waiting for the provider run to end: %v", err)
	}
	return events
}
func JWT(claims jsontext.Value) string {
	encode := base64.RawURLEncoding.EncodeToString
	return encode([]byte(`{"alg":"none","typ":"JWT"}`)) + "." + encode(claims) + ".signature"
}
func SSEBody(payloads ...jsontext.Value) string {
	var body strings.Builder
	for _, payload := range payloads {
		body.WriteString("data: ")
		body.Write(payload)
		body.WriteString("\n\n")
	}
	return body.String()
}

// NextEvent waits for a provider event channel to yield or close.
func NextEvent(t *testing.T, events <-chan provider.ProviderEvent) (provider.ProviderEvent, bool) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	select {
	case event, ok := <-events:
		return event, ok
	case <-ctx.Done():
		t.Fatal("waiting for the provider's next event or its end")
		return nil, false
	}
}

// AssertBuiltInCatalog checks the status and directory of an API-key provider.
func AssertBuiltInCatalog(t *testing.T, p provider.Provider, vendor *MockVendor) {
	t.Helper()
	if p.Capabilities().ProcessHost {
		t.Fatal("an API-key entry starts no process")
	}
	if _, ok := p.AuthStatus(t.Context()).(core.AuthStateAuthenticated); !ok {
		t.Fatal("API-key entry is not authenticated")
	}
	if _, ok := p.RuntimeState().(core.RuntimeStateReady); !ok {
		t.Fatal("API-key entry is not ready")
	}
	list, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if list.Stale || list.SourceFetchedAt != core.UnixEpoch {
		t.Fatal("a built-in directory is never fetched")
	}
	if list.DefaultModelID == nil || !slices.ContainsFunc(list.Models, func(m core.ProviderModel) bool { return m.ID == *list.DefaultModelID }) {
		t.Fatal("the default must be a model in the directory")
	}
	if len(vendor.Requests()) != 0 {
		t.Fatal("reading the status and models sent a request")
	}
}
