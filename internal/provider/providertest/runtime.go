package providertest

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"iter"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// Text builds an answer increment.
func Text(text string) provider.Event { return &provider.TextDelta{Text: text} }

// Thinking builds a reasoning increment.
func Thinking(text string) provider.Event { return &provider.ThinkingDelta{Text: text} }

// ToolCall builds a scripted tool invocation.
func ToolCall(id, name string, input json.RawMessage) provider.Event {
	return &provider.ToolCall{ToolUseID: id, ToolName: name, Input: input}
}

// Response builds a response with input and output usage.
func Response(input, output uint64) provider.Event {
	return &provider.Response{Usage: core.TokenUsage{InputTokens: input, OutputTokens: output}}
}

// Error builds a failure with no diagnostics.
func Error(message string, code *provider.ErrorCode) provider.Event {
	return &provider.Error{Failure: provider.Failure{Message: message, Code: code}}
}

// InferenceRequest returns a minimal hello request for tests to customize.
func InferenceRequest() provider.InferenceRequest {
	return provider.InferenceRequest{
		SessionID: "session-1",
		TurnID:    "turn-1",
		RequestID: "request-1",
		ModelID:   "model-1",
		Items: []provider.InferenceItem{
			&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "hello"}}},
		},
		Tools: []provider.ToolDefinition{},
	}
}

// Turn opens a scripted run. A custom stream must honor ctx when waiting.
type Turn func(context.Context, provider.InferenceRequest) provider.Run

// Events returns a scripted sequence of events.
func Events(events ...provider.Event) Turn {
	return func(ctx context.Context, _ provider.InferenceRequest) provider.Run {
		return func(yield func(provider.Event) bool) {
			for _, event := range events {
				if ctx.Err() != nil || !yield(event) {
					return
				}
			}
		}
	}
}

// Respond computes scripted events from the incoming request.
func Respond(respond func(provider.InferenceRequest) []provider.Event) Turn {
	return func(ctx context.Context, request provider.InferenceRequest) provider.Run {
		return Events(respond(request)...)(ctx, request)
	}
}

// Pending waits until cancellation without emitting anything.
func Pending() Turn {
	return func(ctx context.Context, _ provider.InferenceRequest) provider.Run {
		return func(_ func(provider.Event) bool) { <-ctx.Done() }
	}
}

// ScriptedRuntime and its fresh copies share one ordered script.
type (
	ScriptedRuntime struct{ script *script }
	script          struct {
		mu       sync.Mutex
		t        testing.TB
		turns    []Turn
		requests []provider.InferenceRequest
		closes   int
		limits   provider.RequestLimits
	}
)

// NewScriptedRuntime returns a runtime and registers no background work.
func NewScriptedRuntime(t testing.TB, turns ...Turn) *ScriptedRuntime {
	return &ScriptedRuntime{script: &script{t: t, turns: append([]Turn{}, turns...)}}
}

// SetLimits sets the request limits shared by this script's runtimes.
func (r *ScriptedRuntime) SetLimits(limits provider.RequestLimits) {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	r.script.limits = limits
}

// Requests returns the requests observed so far; their immutable request data is shared.
func (r *ScriptedRuntime) Requests() []provider.InferenceRequest {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return append([]provider.InferenceRequest{}, r.script.requests...)
}

// Remaining returns the number of unplayed turns.
func (r *ScriptedRuntime) Remaining() int {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return len(r.script.turns)
}

// Closes returns how many runtimes of this script were closed.
func (r *ScriptedRuntime) Closes() int {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return r.script.closes
}

// Run consumes one scripted turn, failing the test if it was not scripted.
func (r *ScriptedRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	r.script.mu.Lock()
	r.script.requests = append(r.script.requests, request)
	if len(r.script.turns) == 0 {
		number := len(r.script.requests)
		r.script.mu.Unlock()
		r.script.t.Errorf("ScriptedRuntime: no turn scripted for run #%d", number)
		return Events()(ctx, request)
	}
	turn := r.script.turns[0]
	r.script.turns = r.script.turns[1:]
	r.script.mu.Unlock()
	return func(yield func(provider.Event) bool) {
		for event := range turn(ctx, request) {
			if ctx.Err() != nil || !yield(event) {
				return
			}
		}
	}
}

// Fresh shares the script but no execution state.
func (r *ScriptedRuntime) Fresh() provider.Runtime { return &ScriptedRuntime{script: r.script} }

// Close records the runtime close.
func (r *ScriptedRuntime) Close(context.Context) error {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	r.script.closes++
	return nil
}

// RequestLimits reads this script's configured limits.
func (r *ScriptedRuntime) RequestLimits(core.Model) provider.RequestLimits {
	r.script.mu.Lock()
	defer r.script.mu.Unlock()
	return r.script.limits
}

// Guarded runs context-aware work under a ten-second hang guard. It does not
// create an unowned worker: cancellation unblocks the operation itself.
func Guarded[T any](ctx context.Context, t testing.TB, what string, work func(context.Context) T) T {
	t.Helper()
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	value := work(ctx)
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		t.Fatalf("%s did not come within 10s", what)
	}
	return value
}

// AllEvents collects a context-aware run under the hang guard.
func AllEvents(ctx context.Context, t testing.TB, open func(context.Context) provider.Run) []provider.Event {
	t.Helper()
	return Guarded(ctx, t, "the run's end", func(ctx context.Context) []provider.Event {
		events := make([]provider.Event, 0)
		for event := range open(ctx) {
			events = append(events, event)
		}
		return events
	})
}

// Run executes a request and collects every event with a hang guard.
func Run(
	ctx context.Context,
	t testing.TB,
	runtime provider.Runtime,
	request provider.InferenceRequest,
) []provider.Event {
	t.Helper()
	return AllEvents(ctx, t, func(ctx context.Context) provider.Run { return runtime.Run(ctx, request) })
}

// EventReader reads events one at a time and cancels and stops the iterator on close.
type EventReader struct {
	t      testing.TB
	ctx    context.Context
	cancel context.CancelFunc
	next   func() (provider.Event, bool)
	stop   func()
}

// NewEventReader opens a context-aware run; each NextEvent has its own hang deadline.
func NewEventReader(ctx context.Context, t testing.TB, open func(context.Context) provider.Run) *EventReader {
	ctx, cancel := context.WithCancel(ctx)
	next, stop := iter.Pull(open(ctx))
	reader := &EventReader{t: t, ctx: ctx, cancel: cancel, next: next, stop: stop}
	t.Cleanup(reader.Close)
	return reader
}

// NextEvent returns the next event or false after the run ends.
func (r *EventReader) NextEvent() (provider.Event, bool) {
	r.t.Helper()
	guard, cancel := context.WithTimeout(r.ctx, 10*time.Second)
	defer cancel()
	done := make(chan struct{})
	stop := context.AfterFunc(guard, func() {
		defer close(done)
		r.cancel()
	})
	event, ok := r.next()
	if !stop() {
		<-done
	}
	if errors.Is(guard.Err(), context.DeadlineExceeded) {
		r.t.Fatal("the run's next event did not come within 10s")
	}
	return event, ok
}

// Close cancels the run before joining its iterator.
func (r *EventReader) Close() {
	r.cancel()
	r.stop()
}

// FixedClock always reads the same wall-clock moment.
type FixedClock core.Timestamp

// Now implements core.Clock.
func (c FixedClock) Now() core.Timestamp { return core.Timestamp(c) }

// ManualClock is a settable wall clock; timer tests use testing/synctest instead.
type ManualClock struct {
	mu  sync.Mutex
	now core.Timestamp
}

// NewManualClock returns a clock at start.
func NewManualClock(start core.Timestamp) *ManualClock { return &ManualClock{now: start} }

// Now reads the current moment.
func (c *ManualClock) Now() core.Timestamp {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

// Set moves to an explicit moment.
func (c *ManualClock) Set(to core.Timestamp) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.now = to
}

// Advance moves wall time in either direction.
func (c *ManualClock) Advance(by time.Duration) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	now, err := c.now.Time()
	if err != nil {
		return err
	}
	moved, err := core.TimestampFromTime(now.Add(by))
	if err == nil {
		c.now = moved
	}
	return err
}

// FollowSystem sets the clock to the system time (or synctest time in a bubble).
func (c *ManualClock) FollowSystem() { c.Set(core.SystemClock{}.Now()) }

// JWT creates a token with an unchecked signature for vendor tests.
func JWT(t testing.TB, claims any) string {
	t.Helper()
	encoded, err := contract.EncodeJSON(claims)
	if err != nil {
		t.Fatal(err)
	}
	return base64.RawURLEncoding.EncodeToString(
		[]byte(`{"alg":"none","typ":"JWT"}`),
	) +
		"." +
		base64.RawURLEncoding.EncodeToString(
			encoded,
		) +
		".signature"
}

// SSEBody creates one data frame per JSON payload.
func SSEBody(t testing.TB, payloads ...any) string {
	t.Helper()
	text := ""
	for _, payload := range payloads {
		encoded, err := contract.EncodeJSON(payload)
		if err != nil {
			t.Fatal(err)
		}
		text += "data: " + string(encoded) + "\n\n"
	}
	return text
}
