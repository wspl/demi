package session_test

import (
	"context"
	"errors"
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
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestSteerDuringToolAndWithdrawal(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered, release := make(chan struct{}), make(chan struct{})
		r := toolRuntime("hold", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			select {
			case <-release:
				return textOutcome("done"), nil
			case <-ctx.Done():
				return session.ToolOutcome{}, ctx.Err()
			}
		})
		f := start(t, r, session.DefaultConfig(), tool("hold"), answer("skipping it"))
		a := f.send("run tests", "t1")
		<-entered
		must(t, f.s.Steer(storetest.Text("skip e2e"), "s1"))
		must(t, f.s.Steer(storetest.Text("never mind"), "s2"))
		equal(t, f.s.CancelPendingSteer("s2"), true)
		equal(t, f.s.CancelPendingSteer("s2"), false)
		equal(t, len(f.s.PendingSteers()), 1)
		close(release)
		f.done(a)
		equal(t, steers(f.p.Requests()[1]), []string{"skip e2e"})
		equal(t, len(f.p.Requests()), 2)
		f.history("user", "tool_call:completed", "response", "steer", "text", "response")
	})
}
func TestInputDuringLastStreamContinues(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("done"), providertest.Response(1, 1))
		f := setup(t, turn, answer("noted"))
		a := f.send("go", "t1")
		<-entered
		must(t, f.s.Steer(storetest.Text("add test"), "s1"))
		close(release)
		f.done(a)
		equal(t, len(f.p.Requests()), 2)
		equal(t, steers(f.p.Requests()[1]), []string{"add test"})
		f.history("user", "text", "response", "steer", "text", "response")
	})
}
func TestStreamSteerWrittenBeforeTool(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.ToolCall("call", "look", []byte(`{}`)), providertest.Response(1, 1))
		var f *scenario
		r := toolRuntime("look", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			equal(t, kinds(f.s.Transcript().Blocks), []string{"user", "tool_call:executing", "response", "steer"})
			return textOutcome("seen"), nil
		})
		f = start(t, r, session.DefaultConfig(), turn, answer("done"))
		a := f.send("go", "t1")
		<-entered
		must(t, f.s.Steer(storetest.Text("brief"), "s1"))
		close(release)
		f.done(a)
	})
}
func TestStopWritesSteersAndRefusesIdleSteer(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Pending())
		equal(t, f.s.Steer(storetest.Text("no"), "s0"), error(session.SteerNotRunning))
		a := f.send("go", "t1")
		synctest.Wait()
		must(t, f.s.Steer(storetest.Text("brief"), "s1"))
		_, err := f.s.Abort(t.Context())
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		f.history("user", "steer", "abort")
		equal(t, len(f.s.PendingSteers()), 0)
		equal(t, f.s.Steer(storetest.Text("no"), "s2"), error(session.SteerNotRunning))
	})
}
func TestQueuedMessageBecomesSteer(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("done"), providertest.Response(1, 1))
		f := setup(t, turn, answer("noted"))
		a := f.send("go", "t1")
		<-entered
		queued := f.send("brief", "t2")
		found, err := f.s.SteerQueuedMessage("t2", "s1")
		must(t, err)
		equal(t, found, true)
		end, err := queued.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Dropped)
		equal(t, len(f.s.QueuedMessages()), 0)
		close(release)
		f.done(a)
		equal(t, steers(f.p.Requests()[1]), []string{"brief"})
	})
}
func TestAgentMessagesDuringToolAreDurableAndOrdered(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered, release := make(chan struct{}), make(chan struct{})
		r := toolRuntime("hold", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			select {
			case <-release:
				return textOutcome("done"), nil
			case <-ctx.Done():
				return session.ToolOutcome{}, ctx.Err()
			}
		})
		f := start(t, r, session.DefaultConfig(), tool("hold"), answer("read"))
		a := f.send("go", "t1")
		<-entered
		one, two := message("one"), message("two")
		two.ID = "subagent:child:1"
		two.Event = &core.CompletionEvent{Outcome: "completed"}
		must(t, f.s.AcceptAgentMessage(t.Context(), one))
		must(t, f.s.AcceptAgentMessage(t.Context(), two))
		equal(t, len(f.checkpoint().State.AgentInputs), 2)
		close(release)
		f.done(a)
		equal(t, steers(f.p.Requests()[1]), []string{transcripttest.AgentMessageEnvelope(one), transcripttest.AgentMessageEnvelope(two)})
		equal(t, len(f.checkpoint().State.AgentInputs), 0)
		f.history("user", "tool_call:completed", "response", "agent_message", "agent_message", "text", "response")
	})
}
func TestIdleAgentMessagesOpenOneContinuation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		release := make(chan struct{})
		runtime := &sessiontest.Runtime{Enter: func(ctx context.Context) (*gates.Lease, error) {
			select {
			case <-release:
				return nil, nil
			case <-ctx.Done():
				return nil, ctx.Err()
			}
		}}
		f := start(t, runtime, session.DefaultConfig(), answer("combined"))
		must(t, f.s.AcceptAgentMessage(t.Context(), message("one")))
		must(t, f.s.AcceptAgentMessage(t.Context(), message("two")))
		close(release)
		must(t, f.s.Settled(t.Context()))
		must(t, f.s.AcceptAgentMessage(t.Context(), message("one")))
		must(t, f.s.Settled(t.Context()))
		equal(t, len(f.p.Requests()), 1)
		f.history("agent_message", "agent_message", "text", "response")
		equal(t, len(f.s.QueuedMessages()), 0)
	})
}
func TestRetryFailedAgentContinuation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("task answer"), providertest.Events(providertest.Error("failure", nil)), answer("read result"))
		f.done(f.send("task", "t1"))
		must(t, f.s.AcceptAgentMessage(t.Context(), message("result")))
		must(t, f.s.Settled(t.Context()))
		f.history("user", "text", "response", "agent_message", "error")
		a, err := f.s.Retry()
		must(t, err)
		f.done(a)
		f.history("user", "text", "response", "agent_message", "text", "response")
		req := f.p.Requests()
		equal(t, req[1].TurnID, req[2].TurnID)
	})
}
func TestMessagesAfterStopWaitForUser(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Pending())
		a := f.send("go", "t1")
		synctest.Wait()
		must(t, f.s.AcceptAgentMessage(t.Context(), message("unread")))
		_, err := f.s.Abort(t.Context())
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		must(t, f.s.AcceptAgentMessage(t.Context(), message("late")))
		f.history("user", "abort")
		equal(t, len(f.checkpoint().State.AgentInputs), 2)
		s, _, p := f.restore(f.checkpoint(), answer("continued"))
		s.Wake()
		synctest.Wait()
		equal(t, len(p.Requests()), 0)
		resumed, err := s.Resume()
		must(t, err)
		_, err = resumed.Wait(t.Context())
		must(t, err)
		equal(t, steers(p.Requests()[0]), []string{transcripttest.AgentMessageEnvelope(message("unread")), transcripttest.AgentMessageEnvelope(message("late"))})
	})
}
func TestMessageDuringFinalSaveOpensContinuation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("first"), answer("receipt consumed"))
		gate := f.tree.HoldSaves()
		defer gate.Release()
		a := f.send("work", "t1")
		must(t, gate.Wait(t.Context(), 1))
		equal(t, f.s.Execution(), session.Finalizing)
		done := make(chan error, 1)
		go func() { done <- f.s.AcceptAgentMessage(t.Context(), message("race")) }()
		synctest.Wait()
		gate.Release()
		must(t, <-done)
		f.done(a)
		must(t, f.s.Settled(t.Context()))
		f.history("user", "text", "response", "agent_message", "text", "response")
	})
}
func TestAgentMessageIdentityRefusals(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("read"))
		wrong := message("m1")
		wrong.RecipientID = "elsewhere"
		err := f.s.AcceptAgentMessage(t.Context(), wrong)
		var admission *session.AgentMessageError
		if !errors.As(err, &admission) || admission.Kind != session.AgentMessageRecipient {
			t.Fatal(err)
		}
		empty := message("m2")
		empty.Content = " "
		err = f.s.AcceptAgentMessage(t.Context(), empty)
		if !errors.As(err, &admission) || admission.Kind != session.AgentMessageInvalid {
			t.Fatal(err)
		}
		must(t, f.s.AcceptAgentMessage(t.Context(), message("m3")))
		must(t, f.s.Settled(t.Context()))
		other := message("m3")
		other.Content = "different"
		err = f.s.AcceptAgentMessage(t.Context(), other)
		if !errors.As(err, &admission) || admission.Kind != session.AgentMessageDifferentContent {
			t.Fatal(err)
		}
		equal(t, len(f.p.Requests()), 1)
	})
}
func TestRestoredMessageDeliveredOnce(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("original"))
		f.done(f.send("task", "t1"))
		must(t, f.s.Dispose(t.Context()))
		cp := f.checkpoint()
		cp.State.AgentInputs = []store.PendingAgentInput{{TurnID: "t1", Model: storetest.TestModel(), Message: message("restored")}}
		s, c, p := f.restore(cp, answer("recovered"))
		equal(t, c.Interrupted, false)
		s.Wake()
		must(t, s.Settled(t.Context()))
		must(t, s.AcceptAgentMessage(t.Context(), message("restored")))
		must(t, s.Settled(t.Context()))
		must(t, s.Dispose(t.Context()))
		again, _, later := f.restore(f.checkpoint())
		again.Wake()
		must(t, again.AcceptAgentMessage(t.Context(), message("restored")))
		synctest.Wait()
		equal(t, len(p.Requests()), 1)
		equal(t, len(later.Requests()), 0)
	})
}
func yieldRuntime() *sessiontest.Runtime {
	return toolRuntime("yield", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
		return session.ToolOutcome{Effect: &session.ScheduleYield{DurationMS: 120000}}, nil
	})
}
func TestYieldEndsTurnAndWakesOnce(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := start(t, yieldRuntime(), session.DefaultConfig(), tool("yield"), answer("checked build"))
		f.done(f.send("build", "t1"))
		equal(t, len(f.p.Requests()), 1)
		cp := f.checkpoint()
		equal(t, len(cp.State.Wakeups), 1)
		if cp.State.Wakeups[0].DueAt == nil {
			t.Fatal("not armed")
		}
		time.Sleep(119 * time.Second)
		equal(t, len(f.p.Requests()), 1)
		time.Sleep(2 * time.Second)
		synctest.Wait()
		must(t, f.s.Settled(t.Context()))
		f.history("user", "tool_call:completed", "response", "wakeup", "text", "response")
		equal(t, len(f.checkpoint().State.Wakeups), 0)
		equal(t, f.s.Transcript().Blocks[3].(*core.WakeupBlock).Placement, core.WakeupPlacement("new_turn"))
	})
}
func TestStopCancelsOldestWakeup(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := start(t, yieldRuntime(), session.DefaultConfig(), tool("yield"))
		f.done(f.send("wait", "t1"))
		r, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *r.Target, framewire.AbortTargetPendingYieldWakeup)
		equal(t, r.CanAbortAgain, false)
		must(t, f.s.Flush(t.Context()))
		equal(t, len(f.checkpoint().State.Wakeups), 0)
		time.Sleep(240 * time.Second)
		equal(t, len(f.p.Requests()), 1)
	})
}
func TestWakeupDuringTurnBecomesSteer(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("building"), providertest.Response(1, 1))
		f := start(t, yieldRuntime(), session.DefaultConfig(), tool("yield"), turn, answer("checked"))
		f.done(f.send("build", "t1"))
		a := f.send("else?", "t2")
		<-entered
		time.Sleep(121 * time.Second)
		synctest.Wait()
		equal(t, len(f.s.PendingSteers()), 0)
		close(release)
		f.done(a)
		equal(t, f.s.Transcript().Blocks[6].(*core.WakeupBlock).Placement, core.WakeupPlacement("steer"))
		equal(t, steers(f.p.Requests()[2]), []string{transcripttest.WakeupText})
	})
}
func TestWakeupSurvivesDisposeAndRestore(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := start(t, yieldRuntime(), session.DefaultConfig(), tool("yield"))
		f.done(f.send("wait", "t1"))
		must(t, f.s.Dispose(t.Context()))
		time.Sleep(180 * time.Second)
		s, _, p := f.restore(f.checkpoint(), answer("awake"))
		synctest.Wait()
		must(t, s.Settled(t.Context()))
		equal(t, len(p.Requests()), 1)
		equal(t, kinds(s.Transcript().Blocks[3:]), []string{"wakeup", "text", "response"})
	})
}
func TestInterruptedRestoreHoldsWakeupsAndMessages(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := start(t, yieldRuntime(), session.DefaultConfig(), tool("yield"), providertest.Pending())
		f.done(f.send("wait", "t1"))
		f.send("keep going", "t2")
		synctest.Wait()
		must(t, f.s.AcceptAgentMessage(t.Context(), message("news")))
		cp := f.checkpoint()
		must(t, f.s.Dispose(t.Context()))
		time.Sleep(180 * time.Second)
		s, c, p := f.restore(cp, answer("caught up"))
		equal(t, c.Interrupted, true)
		s.RecordInterruption()
		time.Sleep(5 * time.Second)
		synctest.Wait()
		equal(t, len(p.Requests()), 0)
		a, err := s.Send(storetest.Text("what happened?"), "t3")
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, steers(p.Requests()[0]), []string{transcripttest.AgentMessageEnvelope(message("news")), transcripttest.WakeupText})
	})
}
