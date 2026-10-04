package server_test

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/server/servertest"
	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"go.uber.org/goleak"
)

// All scenarios use in-memory stores and scripted providers; no model or process runs.
func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

type testIDs struct {
	mu   sync.Mutex
	next int
}

func (s *testIDs) NextID() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.next++
	return fmt.Sprintf("id-%d", s.next)
}

type fixture struct {
	t        *testing.T
	server   *server.Server[*toolstest.NoHost]
	store    *storetest.MemoryTreeStore
	resolver *servertest.ScriptedProviders
	script   *providertest.ScriptedRuntime
}

func newFixture(t *testing.T, turns ...providertest.Turn) *fixture {
	return fixtureWith(
		t,
		providertest.NewScriptedRuntime(t, turns...),
		storetest.NewMemoryTreeStore(),
		server.DefaultConfig(),
	)
}

func fixtureWith(
	t *testing.T,
	script *providertest.ScriptedRuntime,
	memory *storetest.MemoryTreeStore,
	config server.Config,
	options ...func(*server.Deps[*toolstest.NoHost]),
) *fixture {
	t.Helper()
	resolver := &servertest.ScriptedProviders{}
	resolver.Provide("stub", script)
	deps := server.Deps[*toolstest.NoHost]{
		Toolsets:     tools.Set{Commands: &host.CommandSet{}, Revision: "test"},
		Instructions: "system prompt",
		Hosts:        &toolstest.NoHost{},
		Shells:       toolstest.NoShells{},
		Providers:    resolver,
		Stores: func(types.NodeID) store.Tree {
			return memory
		},
		Clock:         types.SystemClock{},
		IDs:           &testIDs{},
		Config:        config,
		StatusChanged: func(types.NodeID) {},
	}
	for _, option := range options {
		option(&deps)
	}
	s := server.New(deps)
	t.Cleanup(func() {
		if err := s.Shutdown(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return &fixture{t: t, server: s, store: memory, resolver: resolver, script: script}
}

func rootID() types.NodeID {
	id, err := types.ParseNodeID("conversation")
	if err != nil {
		panic(err)
	}
	return id
}

func turnID(id string) types.TurnID {
	v, err := types.ParseTurnID(id)
	if err != nil {
		panic(err)
	}
	return v
}

func blockID(id string) types.BlockID {
	v, err := types.ParseBlockID(id)
	if err != nil {
		panic(err)
	}
	return v
}

func send(id, text string) *conversationproto.SendFrame {
	return &conversationproto.SendFrame{MessageID: turnID(id), Content: servertest.ClientText(text)}
}

func said(text string) providertest.Turn {
	return providertest.Events(providertest.Text(text), providertest.Response(1, 1))
}

func held(gate <-chan struct{}, events ...provider.Event) providertest.Turn {
	return func(ctx context.Context, request provider.InferenceRequest) provider.Run {
		return func(yield func(provider.Event) bool) {
			select {
			case <-ctx.Done():
				return
			case <-gate:
			}
			providertest.Events(events...)(ctx, request)(yield)
		}
	}
}

func (f *fixture) client() *servertest.TestClient[*toolstest.NoHost] {
	return servertest.Connect(f.t, f.server, rootID(), "/workspace")
}

func (f *fixture) opened() *servertest.TestClient[*toolstest.NoHost] {
	c := f.client()
	c.Send(f.t.Context(), &conversationproto.OpenFrame{})
	frames := c.Received()
	if len(frames) < 5 {
		f.t.Fatalf("handshake: %v", frames)
	}
	if _, ok := frames[0].(*conversationproto.OpenedFrame); !ok {
		f.t.Fatalf("open: %v", frames)
	}
	return c
}

func untilIdle(t *testing.T, c *servertest.TestClient[*toolstest.NoHost]) []conversationproto.ServerFrame {
	t.Helper()
	frames, err := c.NextUntil(t.Context(), func(f conversationproto.ServerFrame) bool {
		phase, ok := f.(*conversationproto.PhaseFrame)
		return ok && phase.Phase == types.SessionPhaseIdle
	})
	if err != nil {
		t.Fatal(err)
	}
	return frames
}

func equal[T any](t *testing.T, want, got T) {
	t.Helper()
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatalf("(-want +got):\n%s", diff)
	}
}

func texts(f *fixture) string {
	var text string
	for _, b := range f.server.Tree(rootID()).Root().Session().Transcript().Blocks {
		if b, ok := b.(*types.TextBlock); ok {
			text += b.Text
		}
	}
	return text
}

func TestOpenHandshakeAndPatchRevisions(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(
			t,
			providertest.Events(providertest.Text("a"), providertest.Text("b"), providertest.Response(1, 1)),
		)
		c := f.client()
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		h := c.Received()
		equal(t, 5, len(h))
		equal(t, "*conversationproto.OpenedFrame", fmt.Sprintf("%T", h[0]))
		reset, ok := h[1].(*conversationproto.TranscriptResetFrame)
		if !ok {
			t.Fatal(h)
		}
		if _, ok := h[2].(*conversationproto.PhaseFrame); !ok {
			t.Fatal(h)
		}
		if _, ok := h[3].(*conversationproto.QueueFrame); !ok {
			t.Fatal(h)
		}
		if _, ok := h[4].(*conversationproto.PendingSteersFrame); !ok {
			t.Fatal(h)
		}
		c.Send(t.Context(), send("m1", "hi"))
		revision := reset.Version.Revision
		for _, frame := range untilIdle(t, c) {
			if patch, ok := frame.(*conversationproto.TranscriptPatchFrame); ok {
				equal(t, revision+1, patch.Revision)
				revision = patch.Revision
			}
		}
		if revision == reset.Version.Revision {
			t.Fatal("no transcript patches")
		}
	})
}

func TestConcurrentOpensBuildOneTree(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t)
		a, b := f.client(), f.client()
		var wg sync.WaitGroup
		wg.Go(func() {
			a.Send(t.Context(), &conversationproto.OpenFrame{})
		})
		wg.Go(func() {
			b.Send(t.Context(), &conversationproto.OpenFrame{})
		})
		wg.Wait()
		equal(t, 1, len(f.resolver.Calls()))
		equal(t, 1, len(f.store.Saves()))
		for _, c := range []*servertest.TestClient[*toolstest.NoHost]{a, b} {
			frames := c.Received()
			equal(t, 5, len(frames))
			for _, frame := range frames {
				if fmt.Sprintf("%T", frame) == "*conversationproto.ClosedFrame" {
					t.Fatal("open closed the other connection")
				}
			}
			if _, ok := frames[0].(*conversationproto.OpenedFrame); !ok {
				t.Fatal(frames)
			}
		}
	})
}

func TestConnectionsShareEventsAndOwnReplies(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("ab"))
		a, b := f.opened(), f.opened()
		a.Send(t.Context(), send("m1", "hi"))
		seen := untilIdle(t, a)
		equal(t, seen, untilIdle(t, b))
		patched := false
		for _, frame := range seen {
			patched = patched || fmt.Sprintf("%T", frame) == "*conversationproto.TranscriptPatchFrame"
		}
		if !patched {
			t.Fatal("no transcript patch reached the clients")
		}
		b.Send(
			t.Context(),
			&conversationproto.SteerFrame{SteerID: blockID("s1"), Content: servertest.ClientText("too late")},
		)
		b.Send(t.Context(), &conversationproto.AbortFrame{})
		b.Send(t.Context(), &conversationproto.SyncTranscriptFrame{})
		command, err := types.ParseCommandID("no-such-command")
		if err != nil {
			t.Fatal(err)
		}
		b.Send(t.Context(), &conversationproto.ShellWriteFrame{CommandID: command, Stdin: "y\n"})
		replies := b.Received()
		equal(t, 4, len(replies))
		for i, kind := range []string{
			"*conversationproto.SteerResultFrame",
			"*conversationproto.AbortResultFrame",
			"*conversationproto.TranscriptResetFrame",
			"*conversationproto.ErrorFrame",
		} {
			equal(t, kind, fmt.Sprintf("%T", replies[i]))
		}
		if _, ok := replies[0].(*conversationproto.SteerResultFrame); !ok {
			t.Fatal(replies)
		}
		equal(t, 0, len(a.Received()))
		a.Send(t.Context(), &conversationproto.CloseFrame{})
		for _, c := range []*servertest.TestClient[*toolstest.NoHost]{a, b} {
			frames := c.Received()
			closed := 0
			for _, frame := range frames {
				if fmt.Sprintf("%T", frame) == "*conversationproto.ClosedFrame" {
					closed++
				}
			}
			equal(t, 1, closed)
			if _, ok := frames[len(frames)-1].(*conversationproto.ClosedFrame); !ok {
				t.Fatal(frames)
			}
		}
		if f.server.Tree(rootID()) != nil {
			t.Fatal("closed tree remains")
		}
	})
}

func TestUnknownProviderLeavesConnectionUnattached(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t)
		f.resolver.Select(storetest.ModelOf("missing", "some-model"))
		c := f.client()
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		c.Send(t.Context(), send("m1", "hi"))
		frames := c.Received()
		equal(t, 2, len(frames))
		e, ok := frames[0].(*conversationproto.ErrorFrame)
		if !ok {
			t.Fatal(frames)
		}
		equal(t, &conversationproto.ErrorFrame{Message: `Provider "missing" is not available`}, e)
		equal(
			t,
			conversationproto.ServerFrame(
				&conversationproto.RejectedFrame{Command: "send", Reason: "No session is open"},
			),
			frames[1],
		)
		if f.server.Tree(rootID()) != nil || f.store.Record(rootID()) != nil {
			t.Fatal("unknown provider created tree")
		}
	})
}

func TestDetachDrainsAndClosesOutbox(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t)
		c, out := f.server.Connect(rootID(), "/workspace", servertest.NewFiles())
		c.Handle(t.Context(), &conversationproto.OpenFrame{})
		tree := f.server.Tree(rootID())
		if !tree.IsAttached() {
			t.Fatal("not attached")
		}
		c.Detach()
		if tree.IsAttached() {
			t.Fatal("still attached")
		}
		for _, kind := range []string{
			"*conversationproto.OpenedFrame",
			"*conversationproto.TranscriptResetFrame",
			"*conversationproto.PhaseFrame",
			"*conversationproto.QueueFrame",
			"*conversationproto.PendingSteersFrame",
		} {
			if !out.Next(t.Context()) {
				t.Fatal(out.Err())
			}
			equal(t, kind, fmt.Sprintf("%T", out.Frame()))
		}
		if out.Next(t.Context()) || out.Err() != nil {
			t.Fatal(out.Frame(), out.Err())
		}
	})
}

func TestDetachedQuiescentTreeIdleEviction(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("done"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "hi"))
		untilIdle(t, c)
		c.Connection().Detach()
		synctest.Wait()
		time.Sleep(5 * time.Minute)
		back := f.opened()
		back.Connection().Detach()
		synctest.Wait()
		time.Sleep(9 * time.Minute)
		if f.server.Tree(rootID()) == nil {
			t.Fatal("evicted early")
		}
		time.Sleep(2 * time.Minute)
		synctest.Wait()
		if f.server.Tree(rootID()) != nil {
			t.Fatal("not evicted")
		}
		cp := f.store.Checkpoint(rootID())
		equal(t, types.SessionPhaseIdle, cp.State.Phase)
		equal(t, 3, len(cp.Transcript))
		equal(t, 1, f.script.Closes())
	})
}

func TestShutdownSavesEveryLiveTree(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending())
		c := f.opened()
		c.Send(t.Context(), send("m1", "hang"))
		synctest.Wait()
		equal(t, 1, len(f.script.Requests()))
		if err := f.server.Shutdown(t.Context()); err != nil {
			t.Fatal(err)
		}
		if f.server.Tree(rootID()) != nil {
			t.Fatal("still live")
		}
		cp := f.store.Checkpoint(rootID())
		equal(t, types.SessionPhaseRunning, cp.State.Phase)
		equal(t, 2, len(cp.Transcript))
		if _, ok := cp.Transcript[0].(*types.UserBlock); !ok {
			t.Fatal(cp.Transcript)
		}
		if _, ok := cp.Transcript[1].(*types.ErrorBlock); !ok {
			t.Fatal(cp.Transcript)
		}
		equal(t, 1, f.script.Closes())
		found := false
		for _, frame := range c.Received() {
			if patch, ok := frame.(*conversationproto.TranscriptPatchFrame); ok {
				for _, p := range patch.Patches {
					if add, ok := p.(*conversationproto.AddPatch); ok {
						if block, ok := add.Value.(*types.ErrorBlock); ok && block.Code != nil &&
							*block.Code == "interrupted" {
							found = true
						}
					}
				}
			}
		}
		if !found {
			t.Fatal("interruption was not published")
		}
	})
}

func TestLaggingConnectionClosesAlone(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		events := make(chan provider.Event)
		script := providertest.NewScriptedRuntime(
			t,
			func(ctx context.Context, _ provider.InferenceRequest) provider.Run {
				return func(yield func(provider.Event) bool) {
					for {
						select {
						case <-ctx.Done():
							return
						case event, ok := <-events:
							if !ok || !yield(event) {
								return
							}
						}
					}
				}
			},
		)
		config := server.DefaultConfig()
		config.OutboxFrames = 16
		f := fixtureWith(t, script, storetest.NewMemoryTreeStore(), config)
		reading, stalled := f.opened(), f.opened()
		reading.Send(t.Context(), send("m1", "count"))
		result := make(chan []conversationproto.ServerFrame, 1)
		go func() {
			frames, err := reading.NextUntil(t.Context(), func(frame conversationproto.ServerFrame) bool {
				p, ok := frame.(*conversationproto.PhaseFrame)
				return ok && p.Phase == types.SessionPhaseIdle
			})
			if err != nil {
				t.Error(err)
			}
			result <- frames
		}()
		for i := range 40 {
			events <- providertest.Text(fmt.Sprintf("%d ", i))
			synctest.Wait()
		}
		events <- providertest.Response(1, 1)
		close(events)
		frames := <-result
		patches := 0
		for _, frame := range frames {
			if _, ok := frame.(*conversationproto.TranscriptPatchFrame); ok {
				patches++
			}
		}
		if patches <= 16 {
			t.Fatal("turn did not exceed outbox", patches)
		}
		_, out := stalled.Split()
		if out.Next(t.Context()) || !errors.Is(out.Err(), server.ErrLagged) {
			t.Fatal(out.Frame(), out.Err())
		}
		if !strings.HasSuffix(texts(f), "39 ") {
			t.Fatal(texts(f))
		}
		if !f.server.Tree(rootID()).IsAttached() {
			t.Fatal("reading connection detached")
		}
	})
}

func TestFramesRefusedWithoutSessionAndWhileBusy(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending())
		c := f.client()
		for _, frame := range []conversationproto.ClientFrame{
			send("m1", "hi"),
			&conversationproto.AbortFrame{},
			&conversationproto.SyncTranscriptFrame{},
			&conversationproto.DequeueMessageFrame{
				MessageID: turnID("m1"),
			},
			&conversationproto.SteerFrame{
				SteerID: blockID("s1"),
				Content: servertest.ClientText("steer"),
			},
			&conversationproto.CancelPendingSteerFrame{
				SteerID: blockID("s1"),
			},
			&conversationproto.AbortSubagentsFrame{},
			&conversationproto.AbortSubagentFrame{
				SubagentID: types.NodeID("child"),
			},
		} {
			c.Send(t.Context(), frame)
		}
		frames := c.Received()
		equal(t, 5, len(frames))
		for i, frame := range frames[:4] {
			r, ok := frame.(*conversationproto.RejectedFrame)
			if !ok {
				t.Fatal(frame)
			}
			equal(
				t,
				[]conversationproto.ClientFrameKind{"send", "abort", "sync_transcript", "dequeue_message"}[i],
				r.Command,
			)
			equal(t, "No session is open", r.Reason)
		}
		equal(
			t,
			conversationproto.ServerFrame(
				&conversationproto.SteerResultFrame{
					SteerID: blockID("s1"),
					Outcome: &conversationproto.RejectedSteer{Reason: "No session is open on this connection"},
				},
			),
			frames[4],
		)
		c.Send(t.Context(), &conversationproto.CloseFrame{})
		equal(t, []conversationproto.ServerFrame{&conversationproto.ClosedFrame{}}, c.Received())
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		c.Received()
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		equal(
			t,
			[]conversationproto.ServerFrame{
				&conversationproto.RejectedFrame{
					Command: "open",
					Reason:  "A session is already open on this connection",
				},
			},
			c.Received(),
		)
		c.Send(t.Context(), send("m1", "hang"))
		synctest.Wait()
		c.Received()
		for _, frame := range []conversationproto.ClientFrame{
			&conversationproto.RetryFrame{},
			&conversationproto.ResumeFrame{},
			&conversationproto.CompactFrame{},
		} {
			c.Send(t.Context(), frame)
			reply := c.Received()
			equal(t, 1, len(reply))
			r := reply[0].(*conversationproto.RejectedFrame)
			equal(t, frame.Kind(), r.Command)
			equal(t, "Session is busy (running)", r.Reason)
		}
	})
}

func TestConnectionsUseOneEditAndQueueAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		busy := make(chan struct{})
		f := newFixture(
			t,
			held(gate, providertest.Text("one"), providertest.Response(1, 1)),
			said("two"),
			said("three"),
			said("one again"),
			held(busy, providertest.Text("busy"), providertest.Response(1, 1)),
		)
		a, b := f.opened(), f.opened()
		a.Send(t.Context(), send("m1", "one"))
		synctest.Wait()
		b.Send(t.Context(), send("m2", "two"))
		a.Send(t.Context(), send("m3", "three"))
		close(gate)
		synctest.Wait()
		seen := a.Received()
		equal(t, seen, b.Received())
		queued := false
		for _, frame := range seen {
			if q, ok := frame.(*conversationproto.QueueFrame); ok && len(q.Queue) == 2 &&
				q.Queue[0].ID == turnID("m2") &&
				q.Queue[1].ID == turnID("m3") {
				queued = true
			}
		}
		if !queued {
			t.Fatal("clients did not see the ordered queue")
		}
		users := []types.TurnID{}
		for _, block := range f.server.Tree(rootID()).Root().Session().Transcript().Blocks {
			if user, ok := block.(*types.UserBlock); ok {
				users = append(users, user.TurnID)
			}
		}
		equal(t, []types.TurnID{turnID("m1"), turnID("m2"), turnID("m3")}, users)
		r := edit(
			"op1",
			userBlock(t, f, "m1"),
			f.server.Tree(rootID()).Root().Session().Transcript().Version,
			"one again",
		)
		saves := f.store.HoldSaves()
		defer saves.Release()
		done := make(chan struct{})
		go func() {
			defer close(done)
			a.Send(t.Context(), r)
		}()
		if err := saves.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		b.Send(t.Context(), send("m4", "during edit"))
		saves.Release()
		<-done
		synctest.Wait()
		refused := false
		for _, frame := range b.Received() {
			if r, ok := frame.(*conversationproto.RejectedFrame); ok {
				equal(t, conversationproto.ClientFrameKind("send"), r.Command)
				equal(t, "A message edit is being prepared", r.Reason)
				refused = true
			}
		}
		if !refused {
			t.Fatal("send during edit accepted")
		}
		accepted := editOutcome(t, a.Received()).(*conversationproto.AcceptedEdit)
		target := userBlock(t, f, string(accepted.TurnID))
		b.Send(t.Context(), send("m5", "busy"))
		synctest.Wait()
		equal(t, 5, len(f.script.Requests()))
		a.Send(
			t.Context(),
			edit("op2", target, f.server.Tree(rootID()).Root().Session().Transcript().Version, "not now"),
		)
		rejectedEdit(t, editOutcome(t, a.Received()), "Message editing requires a settled session with no pending work")
		close(busy)
		untilIdle(t, b)
	})
}

// Pause publication, as a goroutine descheduled between state mutation and its
// callback could be, while a second connection takes the open snapshot.
func TestOpenSnapshotDoesNotReplayEarlierRevisions(t *testing.T) {
	for _, syncTranscript := range []bool{false, true} {
		t.Run(fmt.Sprintf("sync=%t", syncTranscript), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				// Publication can run on the sender while the session worker
				// continues. Hold the provider too, so the snapshot cannot
				// already contain every transcript change of the turn.
				answer := make(chan struct{})
				resume := sync.OnceFunc(func() {
					close(answer)
				})
				defer resume()
				f := newFixture(
					t,
					held(answer, providertest.Text("answer"), providertest.Response(1, 1)),
					said("second"),
				)
				first := f.opened()
				second := f.client()
				if syncTranscript {
					second.Send(t.Context(), &conversationproto.OpenFrame{})
					second.Received()
				}
				agent := f.server.Tree(rootID()).Root().Session()
				paused, release := make(chan struct{}), make(chan struct{})
				unblock := sync.OnceFunc(func() {
					close(release)
				})
				defer unblock()
				var once sync.Once
				subscription := agent.Subscribe(func(event session.Event) {
					if _, ok := event.(*session.TranscriptChanged); ok {
						once.Do(func() {
							close(paused)
							<-release
						})
					}
				})
				defer subscription.Release()
				sent := make(chan struct{})
				go func() {
					defer close(sent)
					first.Send(t.Context(), send("m1", "question"))
				}()
				defer func() {
					unblock()
					resume()
					<-sent
				}()
				<-paused
				agent.RecordInterruption()
				if syncTranscript {
					second.Send(t.Context(), send("m2", "queued"))
				}
				synctest.Wait()
				if syncTranscript {
					second.Send(t.Context(), &conversationproto.SyncTranscriptFrame{})
				} else {
					second.Send(t.Context(), &conversationproto.OpenFrame{})
				}
				var revision uint64
				found := false
				for _, frame := range second.Received() {
					if reset, ok := frame.(*conversationproto.TranscriptResetFrame); ok {
						revision = reset.Version.Revision
						found = true
					}
				}
				unblock()
				resume()
				<-sent
				synctest.Wait()
				if !found {
					t.Fatal("root snapshot missing")
				}
				patches := 0
				queued := false
				for _, frame := range second.Received() {
					if queue, ok := frame.(*conversationproto.QueueFrame); ok {
						for _, message := range queue.Queue {
							if message.ID == turnID("m2") {
								queued = true
							}
						}
					}
					if patch, ok := frame.(*conversationproto.TranscriptPatchFrame); ok {
						equal(t, revision+1, patch.Revision)
						revision = patch.Revision
						patches++
					}
				}
				if syncTranscript && !queued {
					t.Fatal("sync discarded pending queue event")
				}
				if patches == 0 {
					t.Fatal("no events after the snapshot")
				}
				equal(t, agent.Transcript().Version.Revision, revision)
			})
		})
	}
}

// A child's pending patches use the same snapshot boundary on open and sync.
func TestChildSnapshotDoesNotReplayEarlierRevisions(t *testing.T) {
	for _, syncTranscript := range []bool{false, true} {
		t.Run(fmt.Sprintf("sync=%t", syncTranscript), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				start := make(chan struct{})
				script := providertest.NewScriptedRuntime(
					t,
					held(start, providertest.Text("answer"), providertest.Response(1, 1)),
				)
				f := modelFixture(t, []providertest.Turn{said("received")}, childScriptEntry("child", script))
				f.opened()
				id := spawn(t, f, rootID(), `{"prompt":"child"}`)
				synctest.Wait()
				second := f.client()
				if syncTranscript {
					second.Send(t.Context(), &conversationproto.OpenFrame{})
					second.Received()
				}
				agent := f.server.Node(rootID(), id).Session()
				paused, release := make(chan struct{}), make(chan struct{})
				var once sync.Once
				subscription := agent.Subscribe(func(event session.Event) {
					if _, ok := event.(*session.TranscriptChanged); ok {
						once.Do(func() {
							close(paused)
							<-release
						})
					}
				})
				defer subscription.Release()
				close(start)
				<-paused
				agent.RecordInterruption()
				if syncTranscript {
					second.Send(t.Context(), &conversationproto.SyncTranscriptFrame{})
				} else {
					second.Send(t.Context(), &conversationproto.OpenFrame{})
				}
				var revision uint64
				found := false
				for _, frame := range second.Received() {
					if reset, ok := frame.(*conversationproto.SubagentTranscriptResetFrame); ok &&
						reset.SubagentID == id {
						revision = reset.Revision
						found = true
					}
				}
				close(release)
				synctest.Wait()
				if !found {
					t.Fatal("child snapshot missing")
				}
				patches := 0
				for _, frame := range second.Received() {
					if patch, ok := frame.(*conversationproto.SubagentTranscriptPatchFrame); ok &&
						patch.SubagentID == id {
						equal(t, revision+1, patch.Revision)
						revision = patch.Revision
						patches++
					}
				}
				if patches == 0 {
					t.Fatal("no child events after snapshot")
				}
			})
		})
	}
}
