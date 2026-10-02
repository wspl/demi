package session_test

import (
	"context"
	"errors"
	"fmt"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/session/sessiontest"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

type editAnswer struct {
	receipt store.EditReceipt
	err     error
}

func editAsync(t *testing.T, s *session.Session, sub session.EditSubmission) <-chan editAnswer {
	done := make(chan editAnswer, 1)
	go func() {
		r, err := s.EditAndSend(t.Context(), sub)
		done <- editAnswer{r, err}
	}()
	return done
}
func editError(t *testing.T, err error, kind session.EditErrorKind) {
	t.Helper()
	var e *session.EditError
	if !errors.As(err, &e) || e.Kind != kind {
		t.Fatalf("got %v; want edit %v", err, kind)
	}
}
func TestEditSaveIsInvisibleAndIdempotent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("A"), answer("B"), answer("B3"))
		f.done(f.send("A", "A"))
		f.done(f.send("B", "B"))
		before := f.s.Transcript()
		patches := []framewire.TranscriptPatch{}
		sub := f.s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.TranscriptChanged); ok {
				patches = append(patches, e.Patches...)
			}
		})
		defer sub.Release()
		gate := f.tree.HoldSaves()
		defer gate.Release()
		f.tree.FailSaves(1)
		failed := editAsync(t, f.s, edit(f.s, 3, "op1"))
		must(t, gate.Wait(t.Context(), 1))
		equal(t, f.s.Transcript(), before)
		equal(t, len(patches), 0)
		gate.Release()
		editError(t, (<-failed).err, session.EditFailed)
		must(t, f.s.Settled(t.Context()))
		equal(t, f.s.Transcript(), before)
		equal(t, f.p.Closes(), 1)
		gate = f.tree.HoldSaves()
		defer gate.Release()
		submission := edit(f.s, 3, "op2")
		first := editAsync(t, f.s, submission)
		must(t, gate.Wait(t.Context(), 1))
		equal(t, f.s.Transcript(), before)
		equal(t, f.checkpoint().Transcript, before.Blocks)
		check, err := f.s.CheckEdit(submission.OperationID, submission.Digest, submission.Version)
		must(t, err)
		if _, ok := check.(*session.EditInFlight); !ok {
			t.Fatalf("%T", check)
		}
		repeat := editAsync(t, f.s, submission)
		conflict := submission
		conflict.Digest = "other"
		_, err = f.s.EditAndSend(t.Context(), conflict)
		editError(t, err, session.EditConflict)
		_, err = f.s.Send(storetest.Text("C"), "C")
		equal(t, err, error(session.AdmissionEditing))
		equal(t, f.s.Steer(storetest.Text("mind"), "s1"), error(session.SteerEditing))
		_, err = f.s.Retry()
		equal(t, err, error(session.AdmissionEditing))
		_, err = f.s.Compact()
		equal(t, err, error(session.AdmissionEditing))
		_, err = f.s.Storage(t.Context(), &host.StorageRead{Key: "todo"}, store.CommitGuard{})
		if err == nil {
			t.Fatal("storage admitted")
		}
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: storetest.ModelOf("stub", "other-model")}))
		synctest.Wait()
		gate.Release()
		one, two := <-first, <-repeat
		must(t, one.err)
		must(t, two.err)
		equal(t, one.receipt, two.receipt)
		must(t, f.s.Settled(t.Context()))
		f.history("user", "text", "response", "user", "text", "response")
		equal(t, f.s.Transcript().Blocks[:3], before.Blocks[:3])
		equal(t, f.checkpoint().State.Edits, []store.EditReceipt{one.receipt})
		equal(t, f.p.Closes(), 2)
		rewrites := 0
		for _, p := range patches {
			if _, ok := p.(*framewire.ReplacePatch); ok {
				rewrites++
			}
		}
		equal(t, rewrites, 1)
	})
}
func TestStopEditPreparationAndSaveDecision(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var hang atomic.Bool
		r := &sessiontest.Runtime{Before: func(ctx context.Context) (*string, error) {
			if hang.Load() {
				<-ctx.Done()
				return nil, ctx.Err()
			}
			return nil, nil
		}}
		f := start(t, r, session.DefaultConfig(), answer("A"))
		f.done(f.send("A", "A"))
		before := f.s.Transcript()
		hang.Store(true)
		preparing := editAsync(t, f.s, edit(f.s, 0, "op1"))
		synctest.Wait()
		stopped, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *stopped.Target, framewire.AbortTargetActiveTurn)
		editError(t, (<-preparing).err, session.EditStopped)
		must(t, f.s.Settled(t.Context()))
		equal(t, f.s.Transcript(), before)
		hang.Store(false)
		for i, fail := range []bool{true, false} {
			gate := f.tree.HoldSaves()
			defer gate.Release()
			if fail {
				f.tree.FailSaves(1)
			}
			editing := editAsync(t, f.s, edit(f.s, 0, fmt.Sprintf("op%d", i+2)))
			must(t, gate.Wait(t.Context(), 1))
			stop := make(chan framewire.AbortResult, 1)
			go func() {
				r, err := f.s.Abort(t.Context())
				if err != nil {
					t.Error(err)
				}
				stop <- r
			}()
			synctest.Wait()
			select {
			case <-stop:
				t.Fatal("stop passed save")
			default:
			}
			gate.Release()
			decision := <-editing
			result := <-stop
			equal(t, *result.Target, framewire.AbortTargetActiveTurn)
			must(t, f.s.Settled(t.Context()))
			if fail {
				editError(t, decision.err, session.EditFailed)
				equal(t, f.s.Transcript(), before)
			} else {
				must(t, decision.err)
				f.history("user", "abort")
				equal(t, f.checkpoint().State.Edits, []store.EditReceipt{decision.receipt})
			}
		}
		equal(t, len(f.p.Requests()), 1)
	})
}
func TestDisposeWaitsForEditSaveAndKeepsReplacement(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("A"))
		f.done(f.send("A", "A"))
		gate := f.tree.HoldSaves()
		defer gate.Release()
		editing := editAsync(t, f.s, edit(f.s, 0, "op1"))
		must(t, gate.Wait(t.Context(), 1))
		disposed := make(chan error, 1)
		go func() { disposed <- f.s.Dispose(t.Context()) }()
		synctest.Wait()
		equal(t, f.p.Closes(), 0)
		select {
		case <-disposed:
			t.Fatal("dispose passed save")
		default:
		}
		gate.Release()
		decision := <-editing
		must(t, decision.err)
		must(t, <-disposed)
		equal(t, f.p.Closes(), 2)
		equal(t, len(f.p.Requests()), 1)
		cp := f.checkpoint()
		equal(t, kinds(cp.Transcript), []string{"user", "error"})
		equal(t, cp.State.Edits, []store.EditReceipt{decision.receipt})
		equal(t, cp.State.Phase, core.SessionPhase("running"))
	})
}
func TestWakeupRefusesEditAndStillFires(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := start(t, yieldRuntime(), session.DefaultConfig(), tool("yield"), answer("checked"))
		f.done(f.send("build", "A"))
		before := f.s.Transcript()
		_, err := f.s.EditAndSend(t.Context(), edit(f.s, 0, "op1"))
		editError(t, err, session.EditBusy)
		equal(t, f.s.Transcript(), before)
		equal(t, f.s.Status().Wakeups, true)
		time.Sleep(121 * time.Second)
		synctest.Wait()
		must(t, f.s.Settled(t.Context()))
		equal(t, kinds(f.s.Transcript().Blocks[3:]), []string{"wakeup", "text", "response"})
	})
}
func TestPriorSaveCommitsBeforeEditWithoutRestoringRemovedRows(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered := make(chan struct{})
		r := toolRuntime("write", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			<-ctx.Done()
			return session.ToolOutcome{}, ctx.Err()
		})
		f := start(t, r, session.DefaultConfig(), tool("write"))
		f.send("write", "A")
		<-entered
		cp := f.checkpoint()
		must(t, f.s.Dispose(t.Context()))
		turn, streaming, release := gated(providertest.Text("written"), providertest.Response(1, 1))
		s, _, _ := f.restore(cp, turn)
		gate := f.tree.HoldSaves()
		defer gate.Release()
		time.Sleep(2 * time.Second)
		must(t, gate.Wait(t.Context(), 1))
		before := s.Transcript()
		editing := editAsync(t, s, edit(s, 0, "op1"))
		synctest.Wait()
		equal(t, gate.Waiting(), 1)
		equal(t, s.Transcript(), before)
		gate.Release()
		decision := <-editing
		must(t, decision.err)
		<-streaming
		equal(t, kinds(f.checkpoint().Transcript), []string{"user"})
		close(release)
		must(t, s.Settled(t.Context()))
		equal(t, kinds(f.checkpoint().Transcript), []string{"user", "text", "response"})
	})
}
func TestAcceptedEditSurvivesTurnFailureAndResume(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		r := toolRuntime("effect", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			calls++
			return textOutcome("permanent"), nil
		})
		f := start(t, r, session.DefaultConfig(), answer("A"), tool("effect"), providertest.Events(providertest.Error("continuation failed", nil)), answer("recovered"))
		f.done(f.send("A", "A"))
		submission := edit(f.s, 0, "op1")
		receipt, err := f.s.EditAndSend(t.Context(), submission)
		must(t, err)
		must(t, f.s.Settled(t.Context()))
		equal(t, calls, 1)
		f.history("user", "tool_call:completed", "response", "error")
		equal(t, f.checkpoint().State.Edits, []store.EditReceipt{receipt})
		version := f.s.Transcript().Version
		again, err := f.s.EditAndSend(t.Context(), submission)
		must(t, err)
		equal(t, again, receipt)
		equal(t, f.s.Transcript().Version, version)
		equal(t, len(f.p.Requests()), 3)
		a, err := f.s.Resume()
		must(t, err)
		f.done(a)
		equal(t, calls, 1)
	})
}
func TestEditFirstMiddleLastKeepsExactPrefix(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("B1"), providertest.Response(1, 1))
		r := toolRuntime("note", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return textOutcome("noted"), nil
		})
		f := start(t, r, session.DefaultConfig(), tool("note"), answer("A"), turn, answer("B2"), answer("C"))
		f.done(f.send("A", "A"))
		a := f.send("B", "B")
		<-entered
		must(t, f.s.Steer(storetest.Text("mind tests"), "s1"))
		close(release)
		f.done(a)
		f.done(f.send("C", "C"))
		cp := f.checkpoint()
		before := f.s.Transcript().Blocks
		for _, index := range []int{0, 5, 11} {
			s, _, p := f.restore(cp, answer("replacement"))
			receipt, err := s.EditAndSend(t.Context(), edit(s, index, "op1"))
			must(t, err)
			must(t, s.Settled(t.Context()))
			blocks := s.Transcript().Blocks
			equal(t, blocks[:index], before[:index])
			equal(t, kinds(blocks[index:]), []string{"user", "text", "response"})
			equal(t, blocks[index].(*core.UserBlock).TurnID, receipt.TurnID)
			equal(t, p.Requests()[0].Items[len(p.Requests()[0].Items)-1], provider.InferenceItem(&provider.UserMessage{Content: storetest.SentText("replacement")}))
			must(t, s.Dispose(t.Context()))
		}
	})
}
