package commandservice_test

import (
	"context"
	"io"
	"sync"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// No call is refused for how many others are in flight: past any count a new
// call starts, and cancelling calls never ends the connection.

// drain reads a stream to its end; the end of a completed response is not an
// error.
func drain(stream *commandservice.Stream) error {
	for {
		if _, err := stream.Next(); err != nil {
			if err == io.EOF {
				return nil
			}
			return err
		}
	}
}

// endsAndDrains ends the input of a stream, which answers the pull its handler
// waits at, and reads the stream to its end.
func endsAndDrains(stream *commandservice.Stream) error {
	if err := stream.End(); err != nil {
		return err
	}
	return drain(stream)
}

func TestHeldCallsBeyondAnyCountAllStartAndFinish(t *testing.T) {
	server := servicetest.Start(t, operations{"hold": hold, "short": short})
	client := server.Client
	held := make([]*commandservice.Stream, 256)
	for i := range held {
		stream, err := client.Invoke(testContext(t), invocation("hold"))
		if err != nil {
			t.Fatalf("call %d: %v", i, err)
		}
		held[i] = stream
	}
	var finishing sync.WaitGroup
	failures := make(chan error, len(held))
	for _, stream := range held {
		finishing.Add(1)
		go func() {
			defer finishing.Done()
			failures <- endsAndDrains(stream)
		}()
	}
	finishing.Wait()
	close(failures)
	for err := range failures {
		if err != nil {
			t.Fatal(err)
		}
	}
	if got := run(t, client, invocation("short"), nil); string(got.stdout) != "ok" {
		t.Errorf("the call after them printed %q", got.stdout)
	}
}

func TestCancellingACallNeverTurnsAwayTheNext(t *testing.T) {
	server := servicetest.Start(t, operations{"hold": hold, "short": short})
	client := server.Client
	held := make([]*commandservice.Stream, 0, 64)
	open := func() {
		stream, err := client.Invoke(testContext(t), invocation("hold"))
		if err != nil {
			t.Fatal(err)
		}
		held = append(held, stream)
	}
	for range 64 {
		open()
	}
	for range 200 {
		held[0].Cancel()
		held = held[1:]
		open()
	}
	if got := run(t, client, invocation("short"), nil); string(got.stdout) != "ok" {
		t.Errorf("the call after them printed %q", got.stdout)
	}
}

// An output nobody reads fills only its own window: more calls than a 4 MiB
// connection window holds, each stalled with a full stream window of output
// nobody reads, leave the connection to an independent call.
func TestUnreadOutputsNeverHoldBackAnIndependentCall(t *testing.T) {
	ended := newSignal()
	server := servicetest.Start(t, operations{"flood": flood(ended), "short": short})
	client := server.Client
	// 4 MiB is 64 windows of 64 KiB.
	floods := make([]*commandservice.Stream, 80)
	for i := range floods {
		stream, err := client.Invoke(testContext(t), invocation("flood"))
		if err != nil {
			t.Fatal(err)
		}
		// One record read starts the flood; the rest of its output stays
		// unread, in its window.
		if _, err := stream.Next(); err != nil {
			t.Fatal(err)
		}
		floods[i] = stream
	}
	if got := run(t, client, invocation("short"), nil); string(got.stdout) != "ok" {
		t.Errorf("an independent call printed %q beside unread outputs", got.stdout)
	}
	for _, stream := range floods {
		stream.Cancel()
	}
	for range floods {
		ended.receive(t)
	}
}

func TestAbandoningABurstOfCallsKeepsTheConnection(t *testing.T) {
	server := servicetest.Start(t, operations{"hold": hold, "short": short})
	client := server.Client
	stop, stopAll := context.WithCancel(testContext(t))
	var burst sync.WaitGroup
	for range 1000 {
		burst.Add(1)
		go func() {
			defer burst.Done()
			// A call that opens is cancelled at once; one that is still
			// opening when the burst is called off is abandoned mid-request.
			if stream, err := client.Invoke(stop, invocation("hold")); err == nil {
				stream.Cancel()
			}
		}()
	}
	stopAll()
	burst.Wait()
	if got := run(t, client, invocation("short"), nil); string(got.stdout) != "ok" {
		t.Errorf("the call after the burst printed %q", got.stdout)
	}
}
