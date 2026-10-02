package jobs

import (
	"context"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runnerwire"
)

func TestRelayRoutesAnswersAndCancelsLaggingCalls(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		handle, requests := NewConnectionHandle(ctx, make(chan []byte, 1))
		relay := NewRelay(ctx)
		defer func() {
			if err := relay.Close(context.Background()); err != nil {
				t.Error(err)
			}
		}()
		returned := make(chan struct{})
		var first uint64
		var result error
		go func() {
			defer close(returned)
			first, result = handle.Reserve(ctx, "conversation", commandwire.TabSequence, 3)
		}()
		question := (<-requests).(*AskRequest)
		frame, err := relay.Ask(question)
		if err != nil {
			t.Fatal(err)
		}
		message, err := runnerwire.DecodeOutbound(frame)
		if err != nil {
			t.Fatal(err)
		}
		reservation, ok := message.(*runnerwire.NumbersReserve)
		if !ok {
			t.Fatalf("question %T", message)
		}
		answer := uint64(42)
		if !relay.Route(&runnerwire.NumbersReserved{ID: reservation.ID, First: &answer}) {
			t.Fatal("answer not handled")
		}
		<-returned
		if result != nil || first != 42 {
			t.Fatalf("answer %d %v", first, result)
		}
		call, cancelCall := context.WithCancel(ctx)
		defer cancelCall()
		events := make(chan CallEvent, 1)
		if err := handle.RegisterCall(ctx, "call", events, call.Done(), cancelCall); err != nil {
			t.Fatal(err)
		}
		relay.Call((<-requests).(*CallRequest))
		relay.Route(&runnerwire.RPCOutput{CallID: "call", Bytes: []byte("one")})
		relay.Route(&runnerwire.RPCExit{CallID: "call", ExitCode: 7})
		<-call.Done()
		if event := <-events; string(event.(*CallStderr).Bytes) != "one" {
			t.Fatal("first event lost")
		}
		if relay.Route(&runnerwire.Ping{}) {
			t.Fatal("relay claimed unrelated message")
		}
	})
}

func TestBackendAnswerDeadlineStartsAfterQueueAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		handle, requests := NewConnectionHandle(ctx, make(chan []byte, 1))
		for range cap(handle.requests) {
			handle.requests <- &CallRequest{}
		}
		returned := make(chan error, 1)
		go func() {
			_, err := handle.Reserve(ctx, "conversation", commandwire.TabSequence, 1)
			returned <- err
		}()
		synctest.Wait()
		// Waiting for room has no backend-answer deadline in Rust.
		time.Sleep(20 * time.Second)
		select {
		case <-returned:
			t.Fatal("queue wait incorrectly used answer timeout")
		default:
		}
		<-requests
		synctest.Wait()
		admitted := time.Now()
		err := <-returned
		failure, ok := err.(*cmdpkgs.RuntimeError)
		if !ok || failure.Kind != cmdpkgs.Deadline || time.Since(admitted) != 15*time.Second {
			t.Fatalf("deadline kind or timing differs: %T after %v", err, time.Since(admitted))
		}
	})
}
