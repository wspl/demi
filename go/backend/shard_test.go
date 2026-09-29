package backend_test

import (
	"context"
	"errors"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/webapi"
)

func user(t *testing.T, id string) webapi.UserID {
	t.Helper()
	parsed, err := webapi.ParseUserID(id)
	if err != nil {
		t.Fatal(err)
	}
	return parsed
}

// A user's calls run one at a time, in the order they arrived: a call is an
// atomic section of the user's state.
// Cost: in-memory goroutines on fake time.
func TestCallsRunOneAtATimeInArrivalOrder(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		shards := backend.NewShards(&backend.Services{})
		defer shards.Close(context.Background())
		ref := shards.Of(user(t, "ana"))
		var order []int
		for i := range 5 {
			if err := ref.Call(t.Context(), func(*backend.Shard) { order = append(order, i) }); err != nil {
				t.Fatal(err)
			}
		}
		if len(order) != 5 || order[0] != 0 || order[4] != 4 {
			t.Fatalf("order %v", order)
		}
		// Many callers at once never share the section.
		inside, most := 0, 0
		var callers sync.WaitGroup
		for range 50 {
			callers.Go(func() {
				err := ref.Call(t.Context(), func(*backend.Shard) {
					inside++
					most = max(most, inside)
					inside--
				})
				if err != nil {
					t.Error(err)
				}
			})
		}
		callers.Wait()
		if most != 1 {
			t.Fatalf("%d calls ran at once", most)
		}
	})
}

// A panicking call answers ErrCallFailed and the shard serves the next one.
// Cost: in-memory goroutines on fake time.
func TestAPanickingCallFailsAloneAndTheShardGoesOn(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		shards := backend.NewShards(&backend.Services{})
		defer shards.Close(context.Background())
		ref := shards.Of(user(t, "ana"))
		if err := ref.Call(t.Context(), func(*backend.Shard) { panic("broken") }); !errors.Is(err, backend.ErrCallFailed) {
			t.Fatalf("panic answered %v", err)
		}
		ran := false
		if err := ref.Call(t.Context(), func(*backend.Shard) { ran = true }); err != nil || !ran {
			t.Fatalf("next call: %v %v", err, ran)
		}
	})
}

// A requester that leaves withdraws a call still waiting, but a started call
// runs to completion and Call waits for it.
// Cost: in-memory goroutines on fake time.
func TestARequesterThatLeavesStopsOnlyACallNotYetStarted(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		shards := backend.NewShards(&backend.Services{})
		defer shards.Close(context.Background())
		ref := shards.Of(user(t, "ana"))
		// A started call whose requester leaves meanwhile still finishes.
		ctx, leave := context.WithCancel(t.Context())
		finished := false
		err := ref.Call(ctx, func(*backend.Shard) {
			leave()
			finished = true
		})
		if err != nil || !finished {
			t.Fatalf("started call: %v, finished %v", err, finished)
		}
		// A call queued behind another is withdrawn when its requester leaves.
		hold := make(chan struct{})
		held := make(chan struct{})
		go func() {
			// The test holds the shard on purpose to keep the next call queued.
			_ = ref.Call(t.Context(), func(*backend.Shard) {
				close(held)
				<-hold
			})
		}()
		<-held
		queuedCtx, leaveQueue := context.WithCancel(t.Context())
		ran := false
		answered := make(chan error, 1)
		go func() { answered <- ref.Call(queuedCtx, func(*backend.Shard) { ran = true }) }()
		synctest.Wait()
		leaveQueue()
		if err := <-answered; !errors.Is(err, context.Canceled) {
			t.Fatalf("withdrawn call answered %v", err)
		}
		close(hold)
		if err := ref.Call(t.Context(), func(*backend.Shard) {}); err != nil || ran {
			t.Fatalf("withdrawn call ran: %v %v", err, ran)
		}
	})
}

// The close ends the synchronization channels first and then the
// conversation sockets, and the tasks last; meanwhile it refuses calls but
// serves CallWhileClosing, and afterwards it refuses everything, adoption
// included.
// Cost: in-memory goroutines on fake time.
func TestTheCloseEndsTheGroupsInOrderAndServesOnlyWhatItNeeds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		shards := backend.NewShards(&backend.Services{})
		ref := shards.Of(user(t, "ana"))
		releaseSync := make(chan struct{})
		var tasksEnded, socketsEnded bool
		var mu sync.Mutex
		err := ref.Call(t.Context(), func(s *backend.Shard) {
			s.SyncChannels().Go(func(ctx context.Context) {
				<-ctx.Done()
				<-releaseSync
			})
			s.ConversationSockets().Go(func(ctx context.Context) {
				<-ctx.Done()
				mu.Lock()
				socketsEnded = true
				mu.Unlock()
			})
			s.Tasks().Go(func(ctx context.Context) {
				<-ctx.Done()
				mu.Lock()
				tasksEnded = true
				mu.Unlock()
			})
		})
		if err != nil {
			t.Fatal(err)
		}
		closed := make(chan []string, 1)
		go func() { closed <- shards.Close(t.Context()) }()
		synctest.Wait()
		// The close waits for the synchronization channels: the conversation
		// sockets and the tasks still admit work, and only CallWhileClosing is
		// served.
		var socketAdmitted, taskAdmitted bool
		err = ref.CallWhileClosing(t.Context(), func(s *backend.Shard) {
			socketAdmitted = s.ConversationSockets().Go(func(context.Context) {})
			taskAdmitted = s.Tasks().Go(func(context.Context) {})
		})
		if err != nil || !socketAdmitted || !taskAdmitted {
			t.Fatalf("while closing: %v, socket %v, task %v", err, socketAdmitted, taskAdmitted)
		}
		if err := ref.Call(t.Context(), func(*backend.Shard) {}); !errors.Is(err, backend.ErrClosing) {
			t.Fatalf("a call while closing answered %v", err)
		}
		if err := ref.Adopt(t.Context(), func(context.Context, backend.ShardRef) {}); !errors.Is(err, backend.ErrClosing) {
			t.Fatalf("adoption while closing answered %v", err)
		}
		close(releaseSync)
		if failures := <-closed; len(failures) != 0 {
			t.Fatal(failures)
		}
		mu.Lock()
		ended := socketsEnded && tasksEnded
		mu.Unlock()
		if !ended {
			t.Fatal("the close returned before its groups ended")
		}
		for _, call := range []func() error{
			func() error { return ref.Call(t.Context(), func(*backend.Shard) {}) },
			func() error { return ref.CallWhileClosing(t.Context(), func(*backend.Shard) {}) },
			func() error { return shards.Of(user(t, "bob")).Call(t.Context(), func(*backend.Shard) {}) },
		} {
			if err := call(); !errors.Is(err, backend.ErrClosing) {
				t.Fatalf("after the close: %v", err)
			}
		}
	})
}

// Adopted work runs on a goroutine of the shard's tasks, reaches the shard
// through its ref, and ends with the shard's close.
// Cost: in-memory goroutines on fake time.
func TestAdoptedWorkRunsUntilTheShardCloses(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		shards := backend.NewShards(&backend.Services{})
		ref := shards.Of(user(t, "ana"))
		seen := make(chan string, 1)
		ended := make(chan struct{})
		err := ref.Adopt(t.Context(), func(ctx context.Context, ref backend.ShardRef) {
			defer close(ended)
			_ = ref.CallWhileClosing(ctx, func(s *backend.Shard) { seen <- s.User().String() })
			<-ctx.Done()
		})
		if err != nil {
			t.Fatal(err)
		}
		if got := <-seen; got != "ana" {
			t.Fatalf("adopted work saw %q", got)
		}
		shards.Close(t.Context())
		select {
		case <-ended:
		default:
			t.Fatal("the close returned before the adopted work ended")
		}
	})
}
