package session_test

import (
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// These ordering scenarios use memory stores, scripted providers and callback
// barriers in synctest. They consume no wall-clock waiting time.
func TestObserveExcludesPendingCallbacks(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("answer"))
		paused, release := make(chan struct{}), make(chan struct{})
		unblock := sync.OnceFunc(func() { close(release) })
		defer unblock()
		var once sync.Once
		blocker := f.s.Subscribe(func(event session.Event) {
			if _, ok := event.(*session.TranscriptChanged); ok {
				once.Do(func() {
					close(paused)
					<-release
				})
			}
		})
		defer blocker.Release()
		action := f.send("question", "turn")
		<-paused
		f.s.RecordInterruption()
		queued := f.send("later", "queued")
		must(t, f.s.Steer(storetest.Text("remember"), "steer"))
		var events []session.Event
		snapshot, subscription := f.s.Observe(func(event session.Event) { events = append(events, event) })
		defer subscription.Release()
		equal(t, kinds(snapshot.Transcript.Blocks), []string{"user", "error"})
		equal(t, snapshot.Phase, core.SessionPhase("running"))
		equal(t, snapshot.Queue, []core.QueuedMessage{{ID: "queued", Content: storetest.Text("later")}})
		equal(t, snapshot.PendingSteers, []core.PendingSteer{{ID: "steer", TurnID: "turn", Model: storetest.TestModel(), Content: storetest.Text("remember")}})
		f.s.DequeueMessage("queued")
		f.s.CancelPendingSteer("steer")
		unblock()
		f.done(action)
		end, err := queued.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Dropped)
		synctest.Wait()
		revision := snapshot.Transcript.Version.Revision
		queues, steers, phases := 0, 0, 0
		for _, event := range events {
			switch event := event.(type) {
			case *session.TranscriptChanged:
				equal(t, event.Revision, revision+1)
				revision = event.Revision
			case *session.QueueChanged:
				equal(t, len(event.Queue), 0)
				queues++
			case *session.PendingSteersChanged:
				equal(t, len(event.PendingSteers), 0)
				steers++
			case *session.PhaseChanged:
				equal(t, event.Phase, core.SessionPhase("idle"))
				phases++
			case *session.EditCommitted, *session.RetryScheduled, *session.ErrorEvent, *session.ActionFailed:
				t.Errorf("unexpected event %T", event)
			}
		}
		equal(t, revision, f.s.Transcript().Version.Revision)
		equal(t, queues, 1)
		equal(t, steers, 1)
		equal(t, phases, 1)
	})
}

func TestEditAcceptanceIsPublishedBetweenRewriteAndProgress(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("original"), answer("replacement"))
		f.done(f.send("question", "turn"))
		submission := edit(t, f.s, 0, "operation")
		paused, release := make(chan struct{}), make(chan struct{})
		unblock := sync.OnceFunc(func() { close(release) })
		defer unblock()
		var events []session.Event
		blocker := f.s.Subscribe(func(event session.Event) {
			if change, ok := event.(*session.TranscriptChanged); ok {
				if _, ok := change.Patches[0].(*framewire.ReplacePatch); ok {
					close(paused)
					<-release
				}
			}
		})
		defer blocker.Release()
		listener := f.s.Subscribe(func(event session.Event) {
			switch event.(type) {
			case *session.TranscriptChanged, *session.EditCommitted:
				events = append(events, event)
			case *session.ActionFailed, *session.ErrorEvent, *session.PendingSteersChanged, *session.PhaseChanged, *session.QueueChanged, *session.RetryScheduled:
			}
		})
		defer listener.Release()
		gate := f.tree.HoldSaves()
		defer gate.Release()
		first := editAsync(t, f.s, submission)
		must(t, gate.Wait(t.Context(), 1))
		check, err := f.s.CheckEdit(submission.OperationID, submission.Digest, submission.Version)
		must(t, err)
		flight, ok := check.(*session.EditInFlight)
		if !ok {
			t.Fatalf("got %T, want in-flight edit", check)
		}
		shared := make(chan editAnswer, 1)
		go func() {
			receipt, err := flight.Acceptance.Wait(t.Context())
			shared <- editAnswer{receipt, err}
		}()
		gate.Release()
		<-paused
		synctest.Wait()
		equal(t, len(events), 0)
		select {
		case <-first:
			t.Fatal("acceptance resolved before rewrite delivery")
		default:
		}
		unblock()
		decision := <-first
		must(t, decision.err)
		repeated := <-shared
		must(t, repeated.err)
		equal(t, repeated.receipt, decision.receipt)
		must(t, f.s.Settled(t.Context()))
		synctest.Wait()
		if len(events) < 3 {
			t.Fatalf("got %d events, want rewrite, acceptance and progress", len(events))
		}
		rewrite, ok := events[0].(*session.TranscriptChanged)
		if !ok {
			t.Fatalf("first event is %T, want rewrite", events[0])
		}
		if _, ok := rewrite.Patches[0].(*framewire.ReplacePatch); !ok {
			t.Fatalf("first patch is %T, want replace", rewrite.Patches[0])
		}
		accepted, ok := events[1].(*session.EditCommitted)
		if !ok {
			t.Fatalf("event after rewrite is %T, want edit acceptance", events[1])
		}
		equal(t, accepted.Receipt, decision.receipt)
		equal(t, accepted.Receipt.OperationID, submission.OperationID)
		equal(t, accepted.Receipt.TurnID, f.s.Transcript().Blocks[0].(*core.UserBlock).TurnID)
		for _, event := range events[2:] {
			if _, ok := event.(*session.TranscriptChanged); !ok {
				t.Fatalf("extra acceptance: %T", event)
			}
		}
		count := len(events)
		receipt, err := f.s.EditAndSend(t.Context(), submission)
		must(t, err)
		equal(t, receipt, decision.receipt)
		synctest.Wait()
		equal(t, len(events), count)
		equal(t, len(f.p.Requests()), 2)
	})
}
