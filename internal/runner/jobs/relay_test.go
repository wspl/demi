package jobs_test

import (
	"context"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runnerproto"
)

func TestRelayRoutesAnswersAndCancelsLaggingCalls(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		handle, requests := jobs.NewConnection(ctx, make(chan []byte, 1))
		relay := jobs.NewRelay(ctx)
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
			first, result = handle.Reserve(ctx, "conversation", commandproto.TabSequence, 3)
		}()
		question := (<-requests).(*jobs.AskRequest)
		frame, err := relay.Ask(question)
		if err != nil {
			t.Fatal(err)
		}
		message, err := runnerproto.DecodeOutbound(frame)
		if err != nil {
			t.Fatal(err)
		}
		reservation, ok := message.(*runnerproto.NumbersReserve)
		if !ok {
			t.Fatalf("question %T", message)
		}
		answer := uint64(42)
		if !relay.Route(&runnerproto.NumbersReserved{ID: reservation.ID, First: &answer}) {
			t.Fatal("answer not handled")
		}
		<-returned
		if result != nil || first != 42 {
			t.Fatalf("answer %d %v", first, result)
		}
		call, cancelCall := context.WithCancel(ctx)
		defer cancelCall()
		events := make(chan jobs.CallEvent, 1)
		if err := handle.RegisterCall(ctx, "call", events, call.Done(), cancelCall); err != nil {
			t.Fatal(err)
		}
		relay.Call((<-requests).(*jobs.CallRequest))
		relay.Route(&runnerproto.RPCOutput{CallID: "call", Bytes: []byte("one")})
		relay.Route(&runnerproto.RPCExit{CallID: "call", ExitCode: 7})
		<-call.Done()
		if event := <-events; string(event.(*jobs.CallStderr).Bytes) != "one" {
			t.Fatal("first event lost")
		}
		if relay.Route(&runnerproto.Ping{}) {
			t.Fatal("relay claimed unrelated message")
		}
	})
}
