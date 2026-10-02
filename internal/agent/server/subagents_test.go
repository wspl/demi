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

	"github.com/wspl/demi/internal/agent/store"
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
		if u, ok := item.(*provider.UserMessage); ok {
			for _, part := range u.Content {
				if p, ok := part.(*provider.TextPart); ok {
					text.WriteString(p.Text)
					text.WriteByte('\n')
				}
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
	inv := host.RPCInvocation{Path: []string{"demi", "agent", verb}, Argv: []string{}, Args: []byte(args), JSON: json, CWD: "/workspace", Env: map[string]string{}, Caller: &caller, Stdin: true}
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
	id, ok := f.store.Numbered(number)
	if !ok {
		t.Fatal("no child", number)
	}
	return id
}

func TestInheritedChildBriefAndCompletionWakeParent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		childScript := providertest.NewScriptedRuntime(t, said("the file says 42"))
		f := modelFixture(t, []providertest.Turn{said("working"), said("got it")}, childScriptEntry("Read notes.md", childScript))
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
		request := childScript.Requests()[0]
		if strings.Contains(requestText(request), "working") || !strings.Contains(requestText(request), "Read notes.md") {
			t.Fatal(requestText(request))
		}
		closed, receipt := -1, -1
		for i, frame := range c.Received() {
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
		if closed < 0 || receipt <= closed {
			t.Fatalf("close %d receipt %d", closed, receipt)
		}
	})
}
func childScriptEntry(key string, runtime *providertest.ScriptedRuntime) childScript {
	return childScript{key, runtime}
}
func TestChildFailureAndEmptyCompletion(t *testing.T) {
	for _, failure := range []bool{false, true} {
		t.Run(fmt.Sprint(failure), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				turn := providertest.Events(providertest.Response(1, 0))
				phase := store.ClosePhase(&store.Completed{})
				if failure {
					turn = providertest.Events(providertest.Error("refused", nil))
					phase = store.ClosePhase(&store.Failed{Failure: "refused"})
				}
				childScript := providertest.NewScriptedRuntime(t, turn)
				f := modelFixture(t, []providertest.Turn{said("received")}, childScriptEntry("child-task", childScript))
				f.opened()
				child := spawn(t, f, rootID(), `{"prompt":"child-task"}`)
				synctest.Wait()
				record := f.store.Record(child)
				if record.Closed == nil {
					t.Fatal("still live")
				}
				equal(t, phase, record.Closed.Phase)
				if !failure {
					equal(t, "", record.Closed.Phase.(*store.Completed).Result)
				}
				if !record.Delivered {
					t.Fatal("not delivered")
				}
			})
		})
	}
}
func TestChildLimitAndWebAbortAll(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turns := []providertest.Turn{}
		for range 8 {
			turns = append(turns, providertest.Pending())
		}
		script := providertest.NewScriptedRuntime(t, turns...)
		f := modelFixture(t, []providertest.Turn{providertest.Pending()}, childScriptEntry("child-task", script))
		c := f.opened()
		ids := []core.NodeID{}
		for range 8 {
			ids = append(ids, spawn(t, f, rootID(), `{"prompt":"child-task"}`))
		}
		ninth := agent(t, f, rootID(), "spawn", `{"prompt":"child-task"}`)
		equal(t, uint8(1), ninth.code)
		if !strings.Contains(ninth.stderr, "8") {
			t.Fatal(ninth)
		}
		c.Send(t.Context(), &framewire.AbortSubagentsFrame{})
		synctest.Wait()
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
		f := modelFixture(t, []providertest.Turn{held(gate, providertest.Text("busy"), providertest.Response(1, 1)), said("noted first"), said("noted second"), said("noted third")}, childScriptEntry("first task", childScript))
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
		reserved, err := f.server.CommandStorage(t.Context(), rootID(), caller, &host.StorageRead{Key: "agent.start.r1"})
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
		equal(t, "subagentId: 1\n", retried.stdout)
		conflict := agent(t, f, rootID(), "spawn", `{"prompt":"other task","request-id":"r1"}`)
		equal(t, "demi agent spawn: request-id already belongs to different agent arguments\n", conflict.stderr)
		for _, args := range []string{`{"id":1,"message":"second task","request-id":"r2"}`, `{"id":1,"message":"second task","request-id":"r2"}`, `{"id":1,"message":"third task","request-id":"r3"}`} {
			run := agent(t, f, rootID(), "resume", args)
			equal(t, "subagentId: 1\n", run.stdout)
			synctest.Wait()
		}
		equal(t, uint64(3), f.store.Record(child).Round)
		run := agent(t, f, rootID(), "resume", `{"id":1,"message":"second task","request-id":"r2"}`)
		equal(t, "demi agent resume: resume request has been superseded by a later round\n", run.stderr)
		equal(t, 3, len(childScript.Requests()))
	})
}
func TestGrandchildCompletionReachesParentThenRoot(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		parentGate, childGate := make(chan struct{}), make(chan struct{})
		parentScript := providertest.NewScriptedRuntime(t, held(parentGate, providertest.Text("waiting"), providertest.Response(1, 1)), said("parent complete"))
		childScript := providertest.NewScriptedRuntime(t, held(childGate, providertest.Text("child complete"), providertest.Response(1, 1)))
		f := modelFixture(t, []providertest.Turn{said("received parent")}, childScriptEntry("task parent", parentScript), childScriptEntry("task nested", childScript))
		c := f.opened()
		parent := spawn(t, f, rootID(), `{"prompt":"task parent"}`)
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
		equal(t, 1, len(f.script.Requests()))
		equal(t, 2, len(parentScript.Requests()))
		frames := c.Received()
		order := []core.NodeID{}
		for _, frame := range frames {
			if s, ok := frame.(*framewire.SubagentFrame); ok && s.Event == framewire.SubagentEventClosed {
				order = append(order, s.Job.SubagentID)
			}
		}
		equal(t, []core.NodeID{child, parent}, order)
	})
}
func TestDetachedTreeWithChildStaysLive(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		script := providertest.NewScriptedRuntime(t, held(gate, providertest.Text("done"), providertest.Response(1, 1)))
		f := modelFixture(t, []providertest.Turn{said("noted")}, childScriptEntry("task child", script))
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
	})
}

func TestChildYieldKeepsTreeLiveWithoutActionLease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		script := providertest.NewScriptedRuntime(t, held(gate, providertest.ToolCall("wait-1", "yield", []byte(`{"durationMs":60000}`)), providertest.Response(1, 1)), said("waited"))
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
	})
}
func TestRestoreReadsGrandchildrenBeforeSettlingParent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		outerScript := providertest.NewScriptedRuntime(t, held(gate, providertest.Text("delegated"), providertest.Response(1, 1)), said("outer done"))
		innerScript := providertest.NewScriptedRuntime(t, providertest.Pending(), said("inner done"))
		f := modelFixture(t, []providertest.Turn{said("noted")}, childScriptEntry("task outer", outerScript), childScriptEntry("task inner", innerScript))
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
		for _, frame := range c.Received() {
			if s, ok := frame.(*framewire.SubagentFrame); ok && s.Event == framewire.SubagentEventClosed {
				closes = append(closes, s.Job.SubagentID)
			}
		}
		equal(t, []core.NodeID{inner, outer}, closes)
	})
}
func TestAbortClosesSubtreeAndDisposePreservesLiveChildren(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		alphaScript := providertest.NewScriptedRuntime(t, held(gate, providertest.Text("alpha waits"), providertest.Response(1, 1)))
		gammaScript := providertest.NewScriptedRuntime(t, providertest.Pending())
		betaScript := providertest.NewScriptedRuntime(t, providertest.Pending(), said("beta after restart"))
		f := modelFixture(t, []providertest.Turn{said("alpha is gone"), said("beta is back")}, childScriptEntry("task alpha", alphaScript), childScriptEntry("task gamma", gammaScript), childScriptEntry("task beta", betaScript))
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
	})
}

func TestMessagesCrossTreeButLifecycleBelongsToSpawner(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		alphaGate, betaGate := make(chan struct{}), make(chan struct{})
		deltaScript := providertest.NewScriptedRuntime(t, said("delta done"))
		alphaScript := providertest.NewScriptedRuntime(t, held(alphaGate, providertest.Text("alpha waits"), providertest.Response(1, 1)), said("alpha heard"))
		betaScript := providertest.NewScriptedRuntime(t, held(betaGate, providertest.Text("beta first"), providertest.Response(1, 1)), said("beta heard"))
		gammaScript := providertest.NewScriptedRuntime(t, providertest.Pending())
		f := modelFixture(t, []providertest.Turn{said("noted delta"), said("noted beta")}, childScriptEntry("task delta", deltaScript), childScriptEntry("task alpha", alphaScript), childScriptEntry("task beta", betaScript), childScriptEntry("task gamma", gammaScript))
		f.opened()
		spawn(t, f, rootID(), `{"prompt":"task delta","description":"delta"}`)
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
		equal(t, "demi agent send: cannot message your own session\n", agent(t, f, gamma, "send", `{"id":"4","message":"me"}`).stderr)
		equal(t, "demi agent send: the root session has no parent\n", agent(t, f, rootID(), "send", `{"id":"parent","message":"up"}`).stderr)
		for _, r := range []commandRun{agent(t, f, gamma, "send", `{"id":"1","message":"late"}`), agent(t, f, alpha, "abort", `{"id":3}`), agent(t, f, alpha, "resume", `{"id":1,"message":"again"}`), agent(t, f, alpha, "show", `{"id":0}`), agent(t, f, alpha, "show", `{"id":1}`)} {
			equal(t, uint8(1), r.code)
		}
		listed := agent(t, f, gamma, "list", `{}`)
		equal(t, uint8(0), listed.code)
		for _, text := range []string{"alpha", "beta", "gamma", "delta", "← you"} {
			if !strings.Contains(listed.stdout, text) {
				t.Fatal(listed.stdout)
			}
		}
		json, err := agentCall(t.Context(), f, gamma, "list", `{}`, true)
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(json.stdout, `"self":true`) {
			t.Fatal(json.stdout)
		}
		shown := agent(t, f, alpha, "show", `{"id":3}`)
		equal(t, uint8(0), shown.code)
		if !strings.Contains(shown.stdout, "beta") {
			t.Fatal(shown.stdout)
		}
		close(betaGate)
		synctest.Wait()
		equal(t, 2, len(betaScript.Requests()))
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
		reply, err := f.server.CommandStorage(t.Context(), rootID(), caller, &host.StorageRead{Key: "agent.start.pending"})
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
