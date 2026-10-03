package server_test

import (
	"bytes"
	"context"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

// nodeScripts selects a scripted runtime by the child's first brief, as Rust's Model does.
type nodeScripts struct {
	root     *providertest.ScriptedRuntime
	children []childScript
}
type childScript struct {
	key     string
	runtime *providertest.ScriptedRuntime
}

func (m *nodeScripts) Run(ctx context.Context, r provider.InferenceRequest) provider.Run {
	if r.SessionID == string(rootID()) {
		return m.root.Run(ctx, r)
	}
	text := requestText(r)
	for _, child := range m.children {
		if strings.Contains(text, child.key) {
			return child.runtime.Run(ctx, r)
		}
	}
	return m.root.Run(ctx, r)
}
func (m *nodeScripts) Fresh() provider.Runtime                       { return m }
func (*nodeScripts) Close(context.Context) error                     { return nil }
func (*nodeScripts) RequestLimits(core.Model) provider.RequestLimits { return provider.RequestLimits{} }

func requestText(r provider.InferenceRequest) string {
	var text strings.Builder
	for _, item := range r.Items {
		var content []provider.UserPart
		switch u := item.(type) {
		case *provider.UserMessage:
			content = u.Content
		case *provider.UserSteer:
			content = u.Content
		case *provider.AssistantText,
			*provider.AssistantThinking,
			*provider.AssistantRedactedThinking,
			*provider.ToolUse,
			*provider.ToolResult:
		}
		for _, part := range content {
			if p, ok := part.(*provider.TextPart); ok {
				text.WriteString(p.Text)
				text.WriteByte('\n')
			}
		}
	}
	return text.String()
}

func modelFixture(t *testing.T, root []providertest.Turn, children ...childScript) *fixture {
	f := newFixture(t, root...)
	f.resolver.ProvideRuntime("stub", &nodeScripts{root: f.script, children: children})
	return f
}

type nodePort struct {
	mu             sync.Mutex
	fixture        *fixture
	caller         host.JobCaller
	stdout, stderr bytes.Buffer
}

func (p *nodePort) Request(ctx context.Context, request host.PortRequest) (host.PortResponse, error) {
	switch r := request.(type) {
	case *host.PortStdout:
		p.mu.Lock()
		defer p.mu.Unlock()
		p.stdout.Write(r.Bytes)
		return &host.PortWritten{}, nil
	case *host.PortStderr:
		p.mu.Lock()
		defer p.mu.Unlock()
		p.stderr.Write(r.Bytes)
		return &host.PortWritten{}, nil
	case *host.PortReadStdin, *host.PortReadLiveStdin:
		return &host.PortInput{}, nil
	case *host.PortStorage:
		reply, err := p.fixture.server.CommandStorage(ctx, rootID(), p.caller, r.Op)
		return &host.PortStored{Reply: reply}, err
	}
	return nil, fmt.Errorf("unexpected request %T", request)
}

type commandRun struct {
	code           uint8
	stdout, stderr string
}

func agentCall(ctx context.Context, f *fixture, node core.NodeID, verb, args string, json bool) (commandRun, error) {
	live := f.server.Node(rootID(), node)
	if live == nil {
		return commandRun{}, fmt.Errorf("caller %s is not live", node)
	}
	caller := live.JobCaller()
	port := &nodePort{fixture: f, caller: caller}
	inv := host.RPCInvocation{
		Path:   []string{"demi", "agent", verb},
		Argv:   []string{},
		Args:   []byte(args),
		JSON:   json,
		CWD:    "/workspace",
		Env:    map[string]string{},
		Caller: &caller,
		Stdin:  true,
	}
	inv.Context.Conversation = string(rootID())
	code, err := live.Commands().Dispatch(ctx, inv, host.NewRPCPort(port))
	return commandRun{code, port.stdout.String(), port.stderr.String()}, err
}

func agent(t *testing.T, f *fixture, node core.NodeID, verb, args string) commandRun {
	t.Helper()
	run, err := agentCall(t.Context(), f, node, verb, args, false)
	if err != nil {
		t.Fatal(err)
	}
	return run
}

func spawn(t *testing.T, f *fixture, node core.NodeID, args string) core.NodeID {
	t.Helper()
	run := agent(t, f, node, "spawn", args)
	if run.code != 0 {
		t.Fatal(run)
	}
	number, err := strconv.ParseUint(strings.TrimSuffix(strings.TrimPrefix(run.stdout, "subagentId: "), "\n"), 10, 64)
	if err != nil {
		t.Fatal(run)
	}
	equal(t, fmt.Sprintf("subagentId: %d\n", number), run.stdout)
	id, ok := f.store.Numbered(number)
	if !ok {
		t.Fatal("no child", number)
	}
	return id
}

func TestInheritedChildBriefAndCompletionWakeParent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		childScript := providertest.NewScriptedRuntime(t, said("the file says 42"))
		f := modelFixture(
			t,
			[]providertest.Turn{said("working"), said("got it")},
			childScriptEntry("Read notes.md", childScript),
		)
		c := f.opened()
		c.Send(t.Context(), send("m1", "start"))
		untilIdle(t, c)
		empty := agent(t, f, rootID(), "spawn", `{"prompt":" \n "}`)
		equal(t, uint8(1), empty.code)
		equal(t, "demi agent spawn: prompt must not be empty\n", empty.stderr)
		child := spawn(t, f, rootID(), `{"prompt":"Read notes.md and report its content","description":"reader"}`)
		synctest.Wait()
		record := f.store.Record(child)
		if record.Closed == nil {
			t.Fatal("child stayed live")
		}
		equal(t, store.ClosePhase(&store.Completed{Result: "the file says 42"}), record.Closed.Phase)
		if !record.Delivered {
			t.Fatal("completion not delivered")
		}
		equal(t, 2, len(f.script.Requests()))
		equal(t, uint64(1), record.Number)
		equal(t, rootID(), *record.Parent)
		equal(t, "/workspace", f.store.Checkpoint(child).State.CWD)
		equal(t, 1, len(childScript.Requests()))
		request := childScript.Requests()[0]
		equal(t, string(child), request.SessionID)
		if !strings.HasPrefix(request.SystemPrompt, "system prompt\n") ||
			!strings.Contains(request.SystemPrompt, "demi agent spawn") ||
			!strings.Contains(request.SystemPrompt, "demi agent send") {
			t.Fatal(request.SystemPrompt)
		}
		equal(t, 1, len(request.Items))
		parts := request.Items[0].(*provider.UserMessage).Content
		equal(t, 2, len(parts))
		preamble := parts[0].(*provider.TextPart).Text
		if !strings.HasPrefix(preamble, "You are a subagent: agent 1 of this conversation, spawned by agent 0.") ||
			!strings.Contains(preamble, "`demi agent spawn` spawns your own children.") {
			t.Fatal(preamble)
		}
		equal(t, provider.UserPart(&provider.TextPart{Text: "Read notes.md and report its content"}), parts[1])
		answered := f.script.Requests()[1].Items
		steer := answered[len(answered)-1].(*provider.UserSteer)
		equal(t, 1, len(steer.Content))
		envelope := steer.Content[0].(*provider.TextPart).Text
		if !strings.Contains(envelope, `"content":"the file says 42"`) ||
			!strings.Contains(envelope, `"outcome":"completed"`) {
			t.Fatal(envelope)
		}
		receipts := agentReceipts(f, rootID())
		equal(t, 1, len(receipts))
		id, err := (core.CompletionID{Child: child, Round: record.Round}).BlockID()
		if err != nil {
			t.Fatal(err)
		}
		equal(t, id, receipts[0].ID)
		equal(t, "reader", receipts[0].Sender.Description)
		equal(t, core.AgentMessageEvent(&core.CompletionEvent{Outcome: "completed"}), receipts[0].Event)
		equal(t, 0, childScript.Remaining())
		equal(t, 0, f.script.Remaining())
		closed, receipt := -1, -1
		started, reset := false, false
		for i, frame := range c.Received() {
			if event, ok := frame.(*framewire.SubagentFrame); ok {
				switch event.Event {
				case framewire.SubagentEventStarted:
					started = true
					job := event.Job
					equal(t, child, job.SubagentID)
					equal(t, rootID(), job.ParentSessionID)
					equal(t, "reader", job.Description)
					equal(t, (*string)(nil), job.Profile)
					equal(t, framewire.JobPhaseRunning, job.Phase)
					equal(t, (*core.Timestamp)(nil), job.EndedAt)
					equal(t, (*string)(nil), job.Result)
				case framewire.SubagentEventClosed:
					equal(t, framewire.JobPhaseCompleted, event.Job.Phase)
					equal(t, "the file says 42", *event.Job.Result)
					if event.Job.EndedAt == nil {
						t.Fatal("no close time")
					}
				}
			}
			if event, ok := frame.(*framewire.SubagentTranscriptResetFrame); ok && event.SubagentID == child &&
				len(event.Blocks) == 0 {
				reset = true
			}
			if s, ok := frame.(*framewire.SubagentFrame); ok && s.Event == framewire.SubagentEventClosed {
				closed = i
			}
			if p, ok := frame.(*framewire.TranscriptPatchFrame); ok {
				for _, p := range p.Patches {
					if a, ok := p.(*framewire.AddPatch); ok {
						if _, ok := a.Value.(*core.AgentMessageBlock); ok {
							receipt = i
						}
					}
				}
			}
		}
		if !started || !reset {
			t.Fatal("missing start or empty transcript reset")
		}
		if closed < 0 || receipt <= closed {
			t.Fatalf("close %d receipt %d", closed, receipt)
		}
	})
}

func childScriptEntry(key string, runtime *providertest.ScriptedRuntime) childScript {
	return childScript{key, runtime}
}

func TestChildFailureAndEmptyCompletion(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		failing := providertest.NewScriptedRuntime(
			t,
			providertest.Events(providertest.Error("the vendor refused", nil)),
		)
		silent := providertest.NewScriptedRuntime(t, providertest.Events(providertest.Response(1, 1)))
		f := modelFixture(
			t,
			[]providertest.Turn{said("noted"), said("noted")},
			childScriptEntry("task fails", failing),
			childScriptEntry("task silent", silent),
		)
		f.opened()
		first := spawn(t, f, rootID(), `{"prompt":"task fails"}`)
		synctest.Wait()
		second := spawn(t, f, rootID(), `{"prompt":"task silent"}`)
		synctest.Wait()
		equal(t, store.ClosePhase(&store.Failed{Failure: "the vendor refused"}), f.store.Record(first).Closed.Phase)
		equal(t, store.ClosePhase(&store.Completed{}), f.store.Record(second).Closed.Phase)
		receipts := agentReceipts(f, rootID())
		equal(t, 2, len(receipts))
		for i, child := range []core.NodeID{first, second} {
			equal(t, child, receipts[i].Sender.ID)
			equal(t, []string{"the vendor refused", ""}[i], receipts[i].Content)
			equal(
				t,
				core.AgentMessageEvent(
					&core.CompletionEvent{Outcome: []core.CompletionOutcome{"failed", "completed"}[i]},
				),
				receipts[i].Event,
			)
			if !f.store.Record(child).Delivered {
				t.Fatal("completion not delivered")
			}
		}
	})
}

func TestChildLimitAndWebAbortAll(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turns := []providertest.Turn{}
		for range 8 {
			turns = append(turns, providertest.Pending())
		}
		script := providertest.NewScriptedRuntime(t, turns...)
		rootTurns := []providertest.Turn{}
		for range 8 {
			rootTurns = append(rootTurns, said("stopped"))
		}
		f := modelFixture(t, rootTurns, childScriptEntry("child-task", script))
		c := f.opened()
		ids := []core.NodeID{}
		for range 8 {
			ids = append(ids, spawn(t, f, rootID(), `{"prompt":"child-task"}`))
		}
		ninth := agent(t, f, rootID(), "spawn", `{"prompt":"child-task"}`)
		equal(t, uint8(1), ninth.code)
		equal(
			t,
			"demi agent spawn: at most 8 running subagents per session; abort one or wait for a result\n",
			ninth.stderr,
		)
		c.Send(t.Context(), &framewire.AbortSubagentsFrame{})
		synctest.Wait()
		closes := 0
		for _, frame := range c.Received() {
			if event, ok := frame.(*framewire.SubagentFrame); ok && event.Event == framewire.SubagentEventClosed &&
				event.Job.Phase == framewire.JobPhaseAborted {
				closes++
			}
		}
		equal(t, 8, closes)
		equal(t, 8, len(agentReceipts(f, rootID())))
		if !f.server.Tree(rootID()).IsQuiescent() {
			t.Fatal("aborted tree not quiescent")
		}
		for _, id := range ids {
			record := f.store.Record(id)
			if record.Closed == nil {
				t.Fatal("still live", id)
			}
			equal(t, store.ClosePhase(&store.Aborted{}), record.Closed.Phase)
		}
	})
}

func TestStartRetrySurvivesCancellationAndResumeRounds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		childScript := providertest.NewScriptedRuntime(t, said("first done"), said("second done"), said("third done"))
		f := modelFixture(
			t,
			[]providertest.Turn{
				held(gate, providertest.Text("busy"), providertest.Response(1, 1)),
				said("noted first"),
				said("noted second"),
				said("noted third"),
			},
			childScriptEntry("first task", childScript),
		)
		c := f.opened()
		c.Send(t.Context(), send("m1", "work"))
		synctest.Wait()
		ctx, cancel := context.WithCancel(t.Context())
		done := make(chan struct{})
		go func() {
			defer close(done)
			_, _ = agentCall(ctx, f, rootID(), "spawn", `{"prompt":"first task","request-id":"r1"}`, false)
		}()
		synctest.Wait()
		caller := f.server.Node(rootID(), rootID()).JobCaller()
		reserved, err := f.server.CommandStorage(
			t.Context(),
			rootID(),
			caller,
			&host.StorageRead{Key: "agent.start.r1"},
		)
		if err != nil {
			t.Fatal(err)
		}
		if len(reserved.(*host.StorageValue).Value) == 0 {
			t.Fatal("not reserved")
		}
		cancel()
		close(gate)
		<-done
		synctest.Wait()
		child, ok := f.store.Numbered(1)
		if !ok {
			t.Fatal("canceled start was lost")
		}
		equal(t, uint64(1), f.store.Record(child).Round)
		c.Send(t.Context(), &framewire.CloseFrame{})
		c.Received()
		f.opened()
		retried := agent(t, f, rootID(), "spawn", `{"prompt":"first task","request-id":"r1"}`)
		equal(t, uint8(0), retried.code)
		equal(t, "subagentId: 1\n", retried.stdout)
		conflict := agent(t, f, rootID(), "spawn", `{"prompt":"other task","request-id":"r1"}`)
		equal(t, uint8(1), conflict.code)
		equal(t, "demi agent spawn: request-id already belongs to different agent arguments\n", conflict.stderr)
		for _, args := range []string{
			`{"id":1,"message":"second task","request-id":"r2"}`,
			`{"id":1,"message":"second task","request-id":"r2"}`,
			`{"id":1,"message":"third task","request-id":"r3"}`,
		} {
			run := agent(t, f, rootID(), "resume", args)
			equal(t, uint8(0), run.code)
			equal(t, "subagentId: 1\n", run.stdout)
			synctest.Wait()
		}
		equal(t, uint64(3), f.store.Record(child).Round)
		run := agent(t, f, rootID(), "resume", `{"id":1,"message":"second task","request-id":"r2"}`)
		equal(t, uint8(1), run.code)
		equal(t, "demi agent resume: resume request has been superseded by a later round\n", run.stderr)
		equal(t, 3, len(childScript.Requests()))
		second := childScript.Requests()[1]
		seen := requestText(second)
		for _, item := range second.Items {
			if a, ok := item.(*provider.AssistantText); ok {
				seen += a.Text
			}
		}
		if !strings.Contains(seen, "first done") || !strings.Contains(seen, "second task") {
			t.Fatal(seen)
		}
		rounds := []uint64{}
		for _, receipt := range agentReceipts(f, rootID()) {
			rounds = append(rounds, receipt.Sender.Round)
		}
		equal(t, []uint64{1, 2, 3}, rounds)
		equal(t, 0, childScript.Remaining())
		equal(t, 0, f.script.Remaining())
	})
}

func TestGrandchildCompletionReachesParentThenRoot(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		parentGate, childGate := make(chan struct{}), make(chan struct{})
		parentScript := providertest.NewScriptedRuntime(
			t,
			held(parentGate, providertest.Text("waiting"), providertest.Response(1, 1)),
			said("parent complete"),
		)
		childScript := providertest.NewScriptedRuntime(
			t,
			held(childGate, providertest.Text("child complete"), providertest.Response(1, 1)),
		)
		f := modelFixture(
			t,
			[]providertest.Turn{said("heard"), said("received parent")},
			childScriptEntry("task parent", parentScript),
			childScriptEntry("task nested", childScript),
		)
		c := f.opened()
		parent := spawn(t, f, rootID(), `{"prompt":"task parent"}`)
		equal(t, "sent to 0\n", agent(t, f, parent, "send", `{"id":"parent","message":"status: delegating"}`).stdout)
		untilIdle(t, c)
		done := make(chan core.NodeID, 1)
		go func() { done <- spawn(t, f, parent, `{"prompt":"task nested"}`) }()
		synctest.Wait()
		close(parentGate)
		child := <-done
		synctest.Wait()
		if f.store.Record(parent).Closed != nil {
			t.Fatal("parent closed with live child")
		}
		close(childGate)
		synctest.Wait()
		equal(t, parent, *f.store.Record(child).Parent)
		equal(t, store.ClosePhase(&store.Completed{Result: "parent complete"}), f.store.Record(parent).Closed.Phase)
		equal(t, 2, len(f.script.Requests()))
		equal(t, 2, len(parentScript.Requests()))
		frames := c.Received()
		order := []core.NodeID{}
		for _, frame := range frames {
			if s, ok := frame.(*framewire.SubagentFrame); ok && s.Event == framewire.SubagentEventClosed {
				equal(t, framewire.JobPhaseCompleted, s.Job.Phase)
				wantParent := rootID()
				if s.Job.SubagentID == child {
					wantParent = parent
				}
				equal(t, wantParent, s.Job.ParentSessionID)
				order = append(order, s.Job.SubagentID)
			}
		}
		equal(t, []core.NodeID{child, parent}, order)
		if !strings.Contains(requestText(parentScript.Requests()[1]), "child complete") {
			t.Fatal(parentScript.Requests()[1])
		}
		heard := agentReceipts(f, parent)
		equal(t, 1, len(heard))
		equal(t, child, heard[0].Sender.ID)
		received := agentReceipts(f, rootID())
		equal(t, 2, len(received))
		equal(t, core.AgentMessageEvent(&core.MessageEvent{}), received[0].Event)
		equal(t, "status: delegating", received[0].Content)
		equal(t, core.AgentMessageEvent(&core.CompletionEvent{Outcome: "completed"}), received[1].Event)
		equal(t, "parent complete", received[1].Content)
	})
}

func TestDetachedTreeWithChildStaysLive(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		script := providertest.NewScriptedRuntime(t, held(gate, providertest.Text("done"), providertest.Response(1, 1)))
		f := fixtureWith(
			t,
			providertest.NewScriptedRuntime(t, said("noted")),
			storetest.NewMemoryTreeStore(),
			server.DefaultConfig(),
			func(deps *server.Deps[*toolstest.NoHost]) {
				deps.Clock = providertest.FixedClock("2026-09-24T12:00:00.000Z")
			},
		)
		f.resolver.ProvideRuntime(
			"stub",
			&nodeScripts{root: f.script, children: []childScript{childScriptEntry("task child", script)}},
		)
		c := f.opened()
		spawn(t, f, rootID(), `{"prompt":"task child"}`)
		c.Connection().Detach()
		synctest.Wait()
		time.Sleep(11 * time.Minute)
		if f.server.Tree(rootID()) == nil {
			t.Fatal("live child evicted")
		}
		close(gate)
		synctest.Wait()
		time.Sleep(11 * time.Minute)
		synctest.Wait()
		if f.server.Tree(rootID()) != nil {
			t.Fatal("quiet tree not evicted")
		}
		f.opened()
		listed := agent(t, f, rootID(), "list", `{}`)
		equal(t, "● 0  (root session) ← you\n└─○ 1  archived (completed 0s ago)  (no description)\n", listed.stdout)
		equal(t, 1, len(agentReceipts(f, rootID())))
	})
}

func TestChildYieldKeepsTreeLiveWithoutActionLease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		script := providertest.NewScriptedRuntime(
			t,
			held(
				gate,
				providertest.ToolCall("wait-1", "yield", []byte(`{"durationMs":60000}`)),
				providertest.Response(1, 1),
			),
			said("waited"),
		)
		f := modelFixture(t, []providertest.Turn{said("noted")}, childScriptEntry("task wait", script))
		f.opened()
		child := spawn(t, f, rootID(), `{"prompt":"task wait"}`)
		synctest.Wait()
		tree := f.server.Tree(rootID())
		if tree.IsQuiescent() {
			t.Fatal("running child is quiescent")
		}
		if r := tree.Admission().TryReserve(); r != nil {
			r.Release()
			t.Fatal("reserved active tree")
		}
		close(gate)
		synctest.Wait()
		if f.store.Record(child).Closed != nil || tree.IsQuiescent() {
			t.Fatal("yielding child closed")
		}
		reservation := tree.Admission().TryReserve()
		if reservation == nil {
			t.Fatal("yield retained action lease")
		}
		reservation.Release()
		time.Sleep(61 * time.Second)
		synctest.Wait()
		equal(t, store.ClosePhase(&store.Completed{Result: "waited"}), f.store.Record(child).Closed.Phase)
		if !tree.IsQuiescent() {
			t.Fatal("completed tree busy")
		}
		reservation = tree.Admission().TryReserve()
		if reservation == nil {
			t.Fatal("completed tree cannot reserve")
		}
		reservation.Release()
		equal(t, 0, script.Remaining())
		equal(t, 0, f.script.Remaining())
	})
}

func TestRestoreReadsGrandchildrenBeforeSettlingParent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		outerScript := providertest.NewScriptedRuntime(
			t,
			held(gate, providertest.Text("delegated"), providertest.Response(1, 1)),
			said("outer done"),
		)
		innerScript := providertest.NewScriptedRuntime(t, providertest.Pending(), said("inner done"))
		f := modelFixture(
			t,
			[]providertest.Turn{said("noted")},
			childScriptEntry("task outer", outerScript),
			childScriptEntry("task inner", innerScript),
		)
		c := f.opened()
		outer := spawn(t, f, rootID(), `{"prompt":"task outer"}`)
		done := make(chan core.NodeID, 1)
		go func() { done <- spawn(t, f, outer, `{"prompt":"task inner"}`) }()
		synctest.Wait()
		close(gate)
		inner := <-done
		synctest.Wait()
		c.Send(t.Context(), &framewire.CloseFrame{})
		c.Received()
		for _, id := range []core.NodeID{outer, inner} {
			if f.store.Record(id).Closed != nil {
				t.Fatal("dispose closed child")
			}
		}
		reading := f.store.HoldChildrenOf(outer)
		defer reading.Release()
		opened := make(chan struct{})
		go func() {
			defer close(opened)
			c.Send(t.Context(), &framewire.OpenFrame{})
		}()
		if err := reading.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		if f.store.Record(outer).Closed != nil {
			t.Fatal("parent closed before reading children")
		}
		reading.Release()
		<-opened
		synctest.Wait()
		equal(t, store.ClosePhase(&store.Completed{Result: "inner done"}), f.store.Record(inner).Closed.Phase)
		equal(t, store.ClosePhase(&store.Completed{Result: "outer done"}), f.store.Record(outer).Closed.Phase)
		closes := []core.NodeID{}
		events := []string{}
		for _, frame := range c.Received() {
			if event, ok := frame.(*framewire.SubagentFrame); ok {
				events = append(events, string(event.Event)+":"+string(event.Job.SubagentID))
			}
			if s, ok := frame.(*framewire.SubagentFrame); ok && s.Event == framewire.SubagentEventClosed {
				closes = append(closes, s.Job.SubagentID)
			}
		}
		equal(t, []core.NodeID{inner, outer}, closes)
		equal(
			t,
			[]string{
				"started:" + string(outer),
				"started:" + string(inner),
				"closed:" + string(inner),
				"closed:" + string(outer),
			},
			events,
		)
		receipts := agentReceipts(f, rootID())
		equal(t, 1, len(receipts))
		equal(t, outer, receipts[0].Sender.ID)
		equal(t, 0, outerScript.Remaining())
		equal(t, 0, innerScript.Remaining())
		equal(t, 0, f.script.Remaining())
	})
}

func TestAbortClosesSubtreeAndDisposePreservesLiveChildren(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		alphaScript := providertest.NewScriptedRuntime(
			t,
			held(gate, providertest.Text("alpha waits"), providertest.Response(1, 1)),
		)
		gammaScript := providertest.NewScriptedRuntime(t, providertest.Pending())
		betaScript := providertest.NewScriptedRuntime(t, providertest.Pending(), said("beta after restart"))
		f := modelFixture(
			t,
			[]providertest.Turn{said("alpha is gone"), said("beta is back")},
			childScriptEntry("task alpha", alphaScript),
			childScriptEntry("task gamma", gammaScript),
			childScriptEntry("task beta", betaScript),
		)
		c := f.opened()
		alpha := spawn(t, f, rootID(), `{"prompt":"task alpha"}`)
		done := make(chan core.NodeID, 1)
		go func() { done <- spawn(t, f, alpha, `{"prompt":"task gamma"}`) }()
		synctest.Wait()
		close(gate)
		gamma := <-done
		synctest.Wait()
		run := agent(t, f, rootID(), "abort", `{"id":1}`)
		equal(t, "aborted 1\n", run.stdout)
		synctest.Wait()
		for _, id := range []core.NodeID{alpha, gamma} {
			equal(t, store.ClosePhase(&store.Aborted{}), f.store.Record(id).Closed.Phase)
		}
		closes := []core.NodeID{}
		for _, frame := range c.Received() {
			if event, ok := frame.(*framewire.SubagentFrame); ok && event.Event == framewire.SubagentEventClosed {
				equal(t, framewire.JobPhaseAborted, event.Job.Phase)
				closes = append(closes, event.Job.SubagentID)
			}
		}
		equal(t, []core.NodeID{gamma, alpha}, closes)
		gammaBlocks := f.store.Checkpoint(gamma).Transcript
		if _, ok := gammaBlocks[len(gammaBlocks)-1].(*core.AbortBlock); !ok {
			t.Fatal(gammaBlocks)
		}
		equal(t, core.SessionPhaseIdle, f.store.Checkpoint(alpha).State.Phase)
		receipts := agentReceipts(f, rootID())
		equal(t, 1, len(receipts))
		equal(t, "", receipts[0].Content)
		equal(t, core.AgentMessageEvent(&core.CompletionEvent{Outcome: "aborted"}), receipts[0].Event)
		equal(t, 1, len(f.store.Checkpoint(alpha).State.AgentInputs))
		equal(t, 1, len(alphaScript.Requests()))
		if !f.store.Record(gamma).Delivered {
			t.Fatal("nested abort not delivered")
		}
		beta := spawn(t, f, rootID(), `{"prompt":"task beta"}`)
		synctest.Wait()
		c.Send(t.Context(), &framewire.CloseFrame{})
		c.Received()
		if f.store.Record(beta).Closed != nil {
			t.Fatal("dispose archived live child")
		}
		equal(t, core.SessionPhaseRunning, f.store.Checkpoint(beta).State.Phase)
		c.Send(t.Context(), &framewire.OpenFrame{})
		synctest.Wait()
		equal(t, store.ClosePhase(&store.Completed{Result: "beta after restart"}), f.store.Record(beta).Closed.Phase)
		equal(t, 2, len(betaScript.Requests()))
		started := []core.NodeID{}
		for _, frame := range c.Received() {
			if event, ok := frame.(*framewire.SubagentFrame); ok && event.Event == framewire.SubagentEventStarted {
				started = append(started, event.Job.SubagentID)
			}
		}
		equal(t, []core.NodeID{beta}, started)
		equal(t, 0, alphaScript.Remaining())
		equal(t, 0, betaScript.Remaining())
		equal(t, 0, gammaScript.Remaining())
		equal(t, 0, f.script.Remaining())
	})
}

func TestMessagesCrossTreeButLifecycleBelongsToSpawner(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		alphaGate, betaGate := make(chan struct{}), make(chan struct{})
		deltaScript := providertest.NewScriptedRuntime(t, said("delta done"))
		alphaScript := providertest.NewScriptedRuntime(
			t,
			held(alphaGate, providertest.Text("alpha waits"), providertest.Response(1, 1)),
			said("alpha heard"),
		)
		betaScript := providertest.NewScriptedRuntime(
			t,
			held(betaGate, providertest.Text("beta first"), providertest.Response(1, 1)),
			said("beta heard"),
		)
		gammaScript := providertest.NewScriptedRuntime(t, providertest.Pending())
		f := modelFixture(
			t,
			[]providertest.Turn{said("noted delta"), said("noted beta")},
			childScriptEntry("task delta", deltaScript),
			childScriptEntry("task alpha", alphaScript),
			childScriptEntry("task beta", betaScript),
			childScriptEntry("task gamma", gammaScript),
		)
		c := f.opened()
		delta := spawn(t, f, rootID(), `{"prompt":"task delta","description":"delta"}`)
		synctest.Wait()
		alpha := spawn(t, f, rootID(), `{"prompt":"task alpha","description":"alpha"}`)
		beta := spawn(t, f, rootID(), `{"prompt":"task beta","description":"beta"}`)
		done := make(chan core.NodeID, 1)
		go func() { done <- spawn(t, f, alpha, `{"prompt":"task gamma","description":"gamma"}`) }()
		synctest.Wait()
		close(alphaGate)
		gamma := <-done
		synctest.Wait()
		equal(t, "sent to 3\n", agent(t, f, gamma, "send", `{"id":"3","message":"hello from gamma"}`).stdout)
		equal(t, "sent to 2\n", agent(t, f, gamma, "send", `{"id":"parent","message":"gamma status"}`).stdout)
		synctest.Wait()
		equal(t, 2, len(alphaScript.Requests()))
		for i, node := range []core.NodeID{delta, alpha, beta, gamma} {
			equal(t, uint64(i+1), f.store.Record(node).Number)
		}
		heard := requestText(alphaScript.Requests()[1])
		if !strings.Contains(heard, `{"sender":{"agent":4,"description":"gamma","round":1},"event":"message",`) ||
			!strings.Contains(heard, "gamma status") ||
			strings.Contains(heard, "recipientId") {
			t.Fatal(heard)
		}
		self := agent(t, f, gamma, "send", `{"id":"4","message":"me"}`)
		equal(t, uint8(1), self.code)
		equal(t, "demi agent send: cannot message your own session\n", self.stderr)
		up := agent(t, f, rootID(), "send", `{"id":"parent","message":"up"}`)
		equal(t, uint8(1), up.code)
		equal(t, "demi agent send: the root session has no parent\n", up.stderr)
		for i, r := range []commandRun{
			agent(t, f, gamma, "send", `{"id":"1","message":"late"}`),
			agent(t, f, alpha, "abort", `{"id":3}`),
			agent(t, f, alpha, "resume", `{"id":1,"message":"again"}`),
			agent(t, f, alpha, "show", `{"id":0}`),
			agent(t, f, alpha, "show", `{"id":1}`),
		} {
			equal(t, uint8(1), r.code)
			equal(
				t,
				[]string{
					"demi agent send: no live agent \"1\" (see `demi agent list`; an archived child is " +
						"revived only by its parent via resume)\n",
					"demi agent abort: 3 is not one of your running children\n",
					"demi agent resume: no archived subagent 1 (see `demi agent list`)\n",
					"demi agent show: no live agent 0\n",
					"demi agent show: no live agent 1\n",
				}[i],
				r.stderr,
			)
		}
		listed := agent(t, f, gamma, "list", `{}`)
		equal(
			t,
			"● 0  (root session)\n"+
				"├─● 2  running  up 0s  last-event 0s ago  profile=(inherit)  \"alpha\"  execution=idle  "+
				"activity=idle\n"+
				"│ └─● 4  running  up 0s  last-event 0s ago  profile=(inherit)  \"gamma\"  "+
				"execution=provider_streaming  activity=streaming ← you\n"+
				"├─● 3  running  up 0s  last-event 0s ago  profile=(inherit)  \"beta\"  "+
				"execution=provider_streaming  activity=streaming\n"+
				"└─○ 1  archived (completed 0s ago)  \"delta\"\n",
			listed.stdout,
		)
		equal(t, uint8(0), listed.code)
		json, err := agentCall(t.Context(), f, gamma, "list", `{}`, true)
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(json.stdout, `"self":true`) {
			t.Fatal(json.stdout)
		}
		shown := agent(t, f, alpha, "show", `{"id":3}`)
		equal(t, uint8(0), shown.code)
		equal(
			t,
			"id: 3\nparent: 0\ndescription: beta\nprofile: (inherit)\nphase: running\nelapsed: 0s\n"+
				"execution: provider_streaming (for 0s)\nlast-event: 0s ago\nactivity: streaming\n"+
				"last assistant text: (none yet)\n",
			shown.stdout,
		)
		c.Received()
		close(betaGate)
		synctest.Wait()
		equal(t, 2, len(betaScript.Requests()))
		if !strings.Contains(requestText(betaScript.Requests()[1]), "hello from gamma") {
			t.Fatal(betaScript.Requests()[1])
		}
		for _, frame := range c.Received() {
			if event, ok := frame.(*framewire.SubagentFrame); ok && event.Event == framewire.SubagentEventClosed {
				equal(t, beta, event.Job.SubagentID)
			}
		}
		receipts := agentReceipts(f, rootID())
		equal(t, 2, len(receipts))
		equal(t, delta, receipts[0].Sender.ID)
		equal(t, beta, receipts[1].Sender.ID)
		equal(t, store.ClosePhase(&store.Completed{Result: "beta heard"}), f.store.Record(beta).Closed.Phase)
	})
}

// Shutdown must cancel a reserved start waiting for its parent's runtime before
// joining lifecycle work; otherwise neither the start nor the parent can end.
func TestShutdownJoinsStartWaitingOnParent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending())
		c := f.opened()
		c.Send(t.Context(), send("m1", "hold runtime"))
		synctest.Wait()
		done := make(chan struct{})
		go func() {
			defer close(done)
			_, _ = agentCall(t.Context(), f, rootID(), "spawn", `{"prompt":"child","request-id":"pending"}`, false)
		}()
		synctest.Wait()
		caller := f.server.Node(rootID(), rootID()).JobCaller()
		reply, err := f.server.CommandStorage(
			t.Context(),
			rootID(),
			caller,
			&host.StorageRead{Key: "agent.start.pending"},
		)
		if err != nil {
			t.Fatal(err)
		}
		if len(reply.(*host.StorageValue).Value) == 0 {
			t.Fatal("start not reserved")
		}
		if err := f.server.Shutdown(t.Context()); err != nil {
			t.Fatal(err)
		}
		<-done
		equal(t, 1, f.script.Closes())
		if f.server.Tree(rootID()) != nil {
			t.Fatal("shutdown retained tree")
		}
	})
}

// agentReceipts reads the messages delivered into an agent's transcript.
func agentReceipts(f *fixture, node core.NodeID) []core.AgentMessage {
	messages := []core.AgentMessage{}
	var blocks []core.Block
	if node == rootID() {
		blocks = f.server.Tree(rootID()).Root().Session().Transcript().Blocks
	} else {
		blocks = f.store.Checkpoint(node).Transcript
	}
	for _, block := range blocks {
		if receipt, ok := block.(*core.AgentMessageBlock); ok {
			messages = append(messages, receipt.Message)
		}
	}
	return messages
}
