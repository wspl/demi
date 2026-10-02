package pagesync_test

import (
	"context"
	"errors"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// In-memory boundary scenarios; no IO or wall-time waits, under one second.
func TestFanoutAndCoalescing(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var registry pagesync.SyncRegistry
		first := registry.Register("alice", database.TokenHash{})
		defer first.Release()
		slow := registry.Register("alice", database.TokenHash{})
		defer slow.Release()
		other := registry.Register("bob", database.TokenHash{})
		defer other.Release()
		marks := registry.Of("alice")
		want := []pagesync.Part{
			{Kind: pagesync.Conversation, ConversationID: "a"},
			{Kind: pagesync.Conversation, ConversationID: "z"},
			{Kind: pagesync.ConversationOrder}, {Kind: pagesync.Preferences},
			{Kind: pagesync.User}, {Kind: pagesync.Workspaces}, {Kind: pagesync.Devices},
			{Kind: pagesync.Providers}, {Kind: pagesync.Cloud}, {Kind: pagesync.Plugins},
			{Kind: pagesync.Plugin, PluginID: "a"}, {Kind: pagesync.Plugin, PluginID: "z"},
		}
		for range 100 {
			for i := len(want) - 1; i >= 0; i-- {
				marks.Mark(want[i])
			}
		}
		for range 2 {
			if err := first.Marked(t.Context()); err != nil {
				t.Fatal(err)
			}
		}
		if diff := cmp.Diff(want, first.Take().Parts); diff != "" {
			t.Fatal(diff)
		}
		registry.MarkEveryone(pagesync.Part{Kind: pagesync.Providers})
		if diff := cmp.Diff(want, slow.Take().Parts); diff != "" {
			t.Fatal(diff)
		}
		single := pagesync.Marked{Parts: []pagesync.Part{{Kind: pagesync.Providers}}}
		for _, channel := range []*pagesync.Registration{first, other} {
			if diff := cmp.Diff(single, channel.Take()); diff != "" {
				t.Fatal(diff)
			}
			if diff := cmp.Diff(pagesync.Marked{}, channel.Take()); diff != "" {
				t.Fatal(diff)
			}
		}
		first.Release()
		first.Release()
		registry.Mark("alice", pagesync.Part{Kind: pagesync.Cloud})
		registry.EndSession("alice", database.TokenHash{})
		if diff := cmp.Diff(pagesync.Marked{}, first.Take()); diff != "" {
			t.Fatal(diff)
		}
		expected := pagesync.Marked{Parts: []pagesync.Part{{Kind: pagesync.Cloud}}, SessionEnded: true}
		if diff := cmp.Diff(expected, slow.Take()); diff != "" {
			t.Fatal(diff)
		}
		if diff := cmp.Diff(pagesync.Marked{}, other.Take()); diff != "" {
			t.Fatal(diff)
		}
		late := registry.Register("alice", database.TokenHash{})
		defer late.Release()
		if diff := cmp.Diff(pagesync.Marked{}, late.Take()); diff != "" {
			t.Fatal(diff)
		}
	})
}

func TestWaitWakeAndCancellation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var registry pagesync.SyncRegistry
		channel := registry.Register("alice", database.TokenHash{})
		defer channel.Release()
		for _, endSession := range []bool{false, true} {
			ctx, cancel := context.WithCancel(t.Context())
			done := make(chan error, 1)
			go func() { done <- channel.Marked(ctx) }()
			synctest.Wait()
			select {
			case err := <-done:
				t.Fatalf("empty channel woke: %v", err)
			default:
			}
			if endSession {
				registry.EndSession("alice", database.TokenHash{})
			} else {
				registry.Mark("alice", pagesync.Part{Kind: pagesync.Cloud})
			}
			err := <-done
			cancel()
			if err != nil {
				t.Fatal(err)
			}
			channel.Take()
		}
		ctx, cancel := context.WithCancel(t.Context())
		done := make(chan error, 1)
		go func() { done <- channel.Marked(ctx) }()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("cancel: %v", err)
		}
		// Marks while the reader is busy must still be visible to its next wait.
		registry.Mark("alice", pagesync.Part{Kind: pagesync.User})
		if err := channel.Marked(t.Context()); err != nil {
			t.Fatal(err)
		}
	})
}

func TestConcurrentChangesAndRelease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var registry pagesync.SyncRegistry
		retained := registry.Register("alice", database.TokenHash{})
		defer retained.Release()
		var workers sync.WaitGroup
		for range 20 {
			workers.Go(func() {
				channel := registry.Register("alice", database.TokenHash{})
				defer channel.Release()
				registry.MarkEveryone(pagesync.Part{Kind: pagesync.Cloud})
				registry.EndSession("alice", database.TokenHash{})
				channel.Take()
			})
		}
		workers.Wait()
		want := pagesync.Marked{Parts: []pagesync.Part{{Kind: pagesync.Cloud}}, SessionEnded: true}
		if diff := cmp.Diff(want, retained.Take()); diff != "" {
			t.Fatal(diff)
		}
	})
}

// Session isolation is checked in memory with no IO or wall-time waits.
func TestSignOutOnlyWakesMatchingSession(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var registry pagesync.SyncRegistry
		firstSession := database.HashToken("alice-first-session")
		secondSession := database.HashToken("alice-second-session")
		first := registry.Register("alice", firstSession)
		defer first.Release()
		second := registry.Register("alice", secondSession)
		defer second.Release()

		ctx, cancel := context.WithCancel(t.Context())
		var waiters sync.WaitGroup
		defer func() {
			cancel()
			waiters.Wait()
		}()
		firstDone := make(chan error, 1)
		secondDone := make(chan error, 1)
		waiters.Go(func() { firstDone <- first.Marked(ctx) })
		waiters.Go(func() { secondDone <- second.Marked(ctx) })
		synctest.Wait()

		registry.EndSession("alice", firstSession)
		synctest.Wait()
		select {
		case err := <-firstDone:
			if err != nil {
				t.Fatalf("signed-out session wait: %v", err)
			}
		default:
			t.Fatal("signed-out session did not wake")
		}
		select {
		case err := <-secondDone:
			t.Fatalf("other session woke on sign-out: %v", err)
		default:
		}
		if diff := cmp.Diff(pagesync.Marked{SessionEnded: true}, first.Take()); diff != "" {
			t.Fatal(diff)
		}
		if diff := cmp.Diff(pagesync.Marked{}, second.Take()); diff != "" {
			t.Fatal(diff)
		}

		// The other session remains usable and wakes for its own sign-out.
		registry.EndSession("alice", secondSession)
		synctest.Wait()
		select {
		case err := <-secondDone:
			if err != nil {
				t.Fatalf("second session wait: %v", err)
			}
		default:
			t.Fatal("second session did not wake for its own sign-out")
		}
		if diff := cmp.Diff(pagesync.Marked{SessionEnded: true}, second.Take()); diff != "" {
			t.Fatal(diff)
		}
		if diff := cmp.Diff(pagesync.Marked{}, first.Take()); diff != "" {
			t.Fatal(diff)
		}
	})
}
