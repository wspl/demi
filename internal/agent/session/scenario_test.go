package session_test

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/session/sessiontest"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript/transcripttest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// Session scenarios use only memory stores and scripted providers. Every clock
// and persistence/retry/yield timer runs inside synctest; no wall time is spent.
type scenario struct {
	t       *testing.T
	s       *session.Session
	p       *providertest.ScriptedRuntime
	tree    *storetest.MemoryTreeStore
	runtime *sessiontest.Runtime
	config  session.Config
	trace   *runtimeTrace
}

func start(t *testing.T, runtime *sessiontest.Runtime, config session.Config, turns ...providertest.Turn) *scenario {
	t.Helper()
	p := providertest.NewScriptedRuntime(t, turns...)
	tree := storetest.NewMemoryTreeStore()
	trace := &runtimeTrace{}
	s := session.New(
		session.Init{
			ID:      "root",
			CWD:     "/workspace",
			Model:   storetest.TestModel(),
			Runtime: &numberedRuntime{Runtime: p, trace: trace},
		},
		session.Deps{
			Runtime: runtime,
			Store:   tree.Session("root"),
			IDs:     transcripttest.NewSequentialIDs("id"),
			Clock:   core.SystemClock{},
			Config:  config,
		},
	)
	must(t, tree.CreateNode(t.Context(), store.RootRecord("root", core.SystemClock{}.Now()), s.FirstCheckpoint()))
	t.Cleanup(func() { must(t, s.Dispose(context.Background())) })
	return &scenario{t: t, s: s, p: p, tree: tree, runtime: runtime, config: config, trace: trace}
}

func setup(t *testing.T, turns ...providertest.Turn) *scenario {
	config := session.DefaultConfig()
	config.PersistInterval = time.Minute
	return start(t, &sessiontest.Runtime{}, config, turns...)
}

func (f *scenario) send(text, id string) *session.ActionAnswer {
	f.t.Helper()
	a, err := f.s.Send(storetest.Text(text), core.TurnID(id))
	must(f.t, err)
	return a
}

func (f *scenario) done(a *session.ActionAnswer) {
	f.t.Helper()
	end, err := a.Wait(f.t.Context())
	must(f.t, err)
	equal(f.t, end, session.Completed)
}

func (f *scenario) fail(a *session.ActionAnswer) {
	f.t.Helper()
	_, err := a.Wait(f.t.Context())
	if err == nil {
		f.t.Fatal("action succeeded, want failure")
	}
}

func (f *scenario) checkpoint() store.Checkpoint {
	f.t.Helper()
	cp, found, err := f.tree.Session("root").Load(f.t.Context())
	must(f.t, err)
	if !found {
		f.t.Fatal("missing checkpoint")
	}
	return cp
}

func (f *scenario) restore(
	cp store.Checkpoint,
	turns ...providertest.Turn,
) (*session.Session, session.Continuation, *providertest.ScriptedRuntime) {
	return f.restoreWith(cp, &sessiontest.Runtime{}, core.SystemClock{}, turns...)
}

// restoreWith restores a checkpoint with explicit node hooks and clock.
func (f *scenario) restoreWith(
	cp store.Checkpoint,
	runtime *sessiontest.Runtime,
	clock core.Clock,
	turns ...providertest.Turn,
) (*session.Session, session.Continuation, *providertest.ScriptedRuntime) {
	f.t.Helper()
	p := providertest.NewScriptedRuntime(f.t, turns...)
	s, c, err := session.Restore(
		cp,
		"root",
		p,
		session.Deps{
			Runtime: runtime,
			Store:   f.tree.Session("root"),
			IDs:     transcripttest.NewSequentialIDs("restored"),
			Clock:   clock,
			Config:  f.config,
		},
	)
	must(f.t, err)
	f.t.Cleanup(func() { must(f.t, s.Dispose(context.Background())) })
	return s, c, p
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func equal[T any](t *testing.T, got, want T) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v\nwant %#v", got, want)
	}
}

func answer(text string) providertest.Turn {
	return providertest.Events(providertest.Text(text), providertest.Response(1, 1))
}

func tool(name string) providertest.Turn {
	return providertest.Events(providertest.ToolCall("call", name, json.RawMessage(`{}`)), providertest.Response(1, 1))
}

func gated(events ...provider.Event) (providertest.Turn, chan struct{}, chan struct{}) {
	entered, release := make(chan struct{}), make(chan struct{})
	return func(ctx context.Context, r provider.InferenceRequest) provider.Run {
		return func(yield func(provider.Event) bool) {
			close(entered)
			select {
			case <-release:
			case <-ctx.Done():
				return
			}
			for e := range providertest.Events(events...)(ctx, r) {
				if !yield(e) {
					return
				}
			}
		}
	}, entered, release
}

func hanging(events ...provider.Event) providertest.Turn {
	return func(ctx context.Context, r provider.InferenceRequest) provider.Run {
		return func(yield func(provider.Event) bool) {
			for e := range providertest.Events(events...)(ctx, r) {
				if !yield(e) {
					return
				}
			}
			<-ctx.Done()
		}
	}
}

func toolRuntime(
	name string,
	invoke func(context.Context, session.ToolInvocation) (session.ToolOutcome, error),
) *sessiontest.Runtime {
	return &sessiontest.Runtime{
		Definitions: []provider.ToolDefinition{{Name: name, InputSchema: json.RawMessage(`{"type":"object"}`)}},
		Invoke:      invoke,
	}
}

func textOutcome(text string) session.ToolOutcome {
	return session.ToolOutcome{Output: []provider.ResultPart{&provider.TextPart{Text: text}}}
}

func message(id string) core.AgentMessage {
	return core.AgentMessage{
		ID:          core.BlockID(id),
		Sender:      core.Sender{ID: "child", Number: 1, Description: "worker", Round: 1},
		RecipientID: "root",
		Timestamp:   core.UnixEpoch,
		Content:     id,
		Event:       &core.MessageEvent{},
	}
}

func kinds(blocks []core.Block) []string {
	out := []string{}
	for _, b := range blocks {
		name := ""
		switch b := b.(type) {
		case *core.UserBlock:
			name = "user"
		case *core.ContextBlock:
			name = "context"
		case *core.SteerBlock:
			name = "steer"
		case *core.AgentMessageBlock:
			name = "agent_message"
		case *core.WakeupBlock:
			name = "wakeup"
		case *core.TextBlock:
			name = "text"
		case *core.ThinkingBlock:
			name = "thinking"
		case *core.RedactedThinkingBlock:
			name = "redacted_thinking"
		case *core.ResponseBlock:
			name = "response"
		case *core.ToolCallBlock:
			name = "tool_call:" + string(b.Status)
		case *core.ErrorBlock:
			name = "error"
		case *core.AbortBlock:
			name = "abort"
		case *core.ResumeBlock:
			name = "resume"
		case *core.CompactionBoundaryBlock:
			name = "compaction_boundary"
		case *core.CompactionMarkerBlock:
			name = "compaction_marker"
		}
		out = append(out, name)
	}
	return out
}

func (f *scenario) history(want ...string) {
	f.t.Helper()
	equal(f.t, kinds(f.s.Transcript().Blocks), want)
}

func steers(t *testing.T, r provider.InferenceRequest) []string {
	t.Helper()
	out := []string{}
	for _, item := range r.Items {
		if item, ok := item.(*provider.UserSteer); ok {
			equal(t, len(item.Content), 1)
			part, ok := item.Content[0].(*provider.TextPart)
			if !ok {
				t.Fatalf("steer content: %T", item.Content[0])
			}
			out = append(out, part.Text)
		}
	}
	return out
}

func edit(t *testing.T, s *session.Session, target int, id string) session.EditSubmission {
	t.Helper()
	snap := s.Transcript()
	digest, err := session.EditDigest(
		framewire.EditRequest{
			OperationID:   core.OperationID(id),
			TargetBlockID: snap.Blocks[target].ID(),
			Version:       snap.Version,
			Content:       []framewire.ClientContent{&framewire.TextContent{Text: "replacement"}},
		},
	)
	must(t, err)
	return session.EditSubmission{
		OperationID: core.OperationID(id),
		Target:      snap.Blocks[target].ID(),
		Version:     snap.Version,
		Digest:      digest,
		Content:     []session.EditContent{&session.Content{Block: &core.UserText{Text: "replacement"}}},
	}
}

func TestRepeatedMessageID(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("once"))
		a := f.send("hello", "stable")
		end, err := f.send("hello", "stable").Wait(t.Context())
		must(t, err)
		equal(t, end, session.Duplicate)
		f.done(a)
		end, err = f.send("hello", "stable").Wait(t.Context())
		must(t, err)
		equal(t, end, session.Duplicate)
		equal(t, len(f.p.Requests()), 1)
	})
}

func TestReentrantEvents(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Pending())
		seen := []string{}
		first := f.s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.QueueChanged); ok && len(e.Queue) > 0 {
				f.s.DequeueMessage(e.Queue[0].ID)
			}
		})
		defer first.Release()
		second := f.s.Subscribe(func(e session.Event) {
			switch e := e.(type) {
			case *session.QueueChanged:
				seen = append(seen, fmt.Sprintf("queue %d", len(e.Queue)))
			case *session.PhaseChanged:
				seen = append(seen, "phase "+string(e.Phase))
			case *session.ActionFailed,
				*session.ErrorEvent,
				*session.PendingSteersChanged,
				*session.RetryScheduled,
				*session.TranscriptChanged,
				*session.EditCommitted:
			}
		})
		defer second.Release()
		f.send("run", "t1")
		f.send("queued", "t2")
		synctest.Wait()
		equal(t, seen, []string{"phase running", "queue 1", "queue 0"})
	})
}

// itemKinds observes the ordered roles sent to the provider.
func itemKinds(items []provider.InferenceItem) []string {
	result := make([]string, 0, len(items))
	for _, item := range items {
		switch item.(type) {
		case *provider.UserMessage:
			result = append(result, "user_message")
		case *provider.UserSteer:
			result = append(result, "user_steer")
		case *provider.AssistantThinking:
			result = append(result, "assistant_thinking")
		case *provider.AssistantText:
			result = append(result, "assistant_text")
		case *provider.AssistantRedactedThinking:
			result = append(result, "assistant_redacted_thinking")
		case *provider.ToolUse:
			result = append(result, "tool_use")
		case *provider.ToolResult:
			result = append(result, "tool_result")
		default:
			result = append(result, fmt.Sprintf("%T", item))
		}
	}
	return result
}

// runtimeTrace records which session runtime serves and closes each request.
type runtimeTrace struct {
	mu     sync.Mutex
	next   int
	served []int
	closed []int
}
type numberedRuntime struct {
	provider.Runtime
	trace *runtimeTrace
	id    int
}

func (r *numberedRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	r.trace.mu.Lock()
	r.trace.served = append(r.trace.served, r.id)
	r.trace.mu.Unlock()
	return r.Runtime.Run(ctx, request)
}

func (r *numberedRuntime) Fresh() provider.Runtime {
	r.trace.mu.Lock()
	r.trace.next++
	id := r.trace.next
	r.trace.mu.Unlock()
	return &numberedRuntime{Runtime: r.Runtime.Fresh(), trace: r.trace, id: id}
}

func (r *numberedRuntime) Close(ctx context.Context) error {
	r.trace.mu.Lock()
	r.trace.closed = append(r.trace.closed, r.id)
	r.trace.mu.Unlock()
	return r.Runtime.Close(ctx)
}

func (r *runtimeTrace) assert(t *testing.T, served, closed []int) {
	t.Helper()
	r.mu.Lock()
	defer r.mu.Unlock()
	equal(t, r.served, served)
	equal(t, r.closed, closed)
}
