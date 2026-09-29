package backend_test

import (
	"context"
	"errors"
	"slices"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/webapi"
)

// A page's channel takes the parts marked for its user, each once and in the
// order they are sent, whether they were marked before it waited or while it
// waits; signing a session out ends only that session's channels; and a
// closed channel gets nothing more (backend.md § Browser synchronization).
// Cost: in memory, on fake time.
func TestAChannelTakesEachMarkedPartOnceInOrder(t *testing.T) {
	synctest.Test(t, channelTakesEachMarkedPartOnceInOrder)
}

func channelTakesEachMarkedPartOnceInOrder(t *testing.T) {
	var registry backend.SyncRegistry
	ana, bo := user(t, "ana"), user(t, "bo")
	session, other := storage.HashToken("session-1"), storage.HashToken("session-2")
	first := registry.Register(ana, session)
	second := registry.Register(ana, other)
	third := registry.Register(bo, session)
	defer second.Close()
	defer third.Close()
	conversation := must(webapi.ParseConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01"))
	registry.Mark(ana, backend.SyncProviders)
	registry.Mark(ana, backend.SyncConversationOrder)
	registry.Mark(ana, backend.SyncConversation(conversation))
	registry.Mark(ana, backend.SyncProviders)
	if err := first.Marked(t.Context()); err != nil {
		t.Fatal(err)
	}
	sent := []backend.SyncPart{backend.SyncConversation(conversation), backend.SyncConversationOrder, backend.SyncProviders}
	if marked := first.Take(); !slices.Equal(marked.Parts, sent) || marked.SessionEnded {
		t.Fatalf("took %+v", marked)
	}
	if marked := third.Take(); len(marked.Parts) != 0 {
		t.Fatalf("another user's channel took %+v", marked)
	}

	registry.MarkEveryone(backend.SyncProviders)
	registry.EndSession(ana, session)
	takes := map[string]struct {
		channel *backend.SyncRegistration
		marked  backend.SyncMarked
	}{
		"the ended session's channel": {first, backend.SyncMarked{Parts: []backend.SyncPart{backend.SyncProviders}, SessionEnded: true}},
		"the user's other channel":    {second, backend.SyncMarked{Parts: sent}},
		"another user's channel":      {third, backend.SyncMarked{Parts: []backend.SyncPart{backend.SyncProviders}}},
	}
	for name, take := range takes {
		if marked := take.channel.Take(); !slices.Equal(marked.Parts, take.marked.Parts) || marked.SessionEnded != take.marked.SessionEnded {
			t.Errorf("%s took %+v, want %+v", name, marked, take.marked)
		}
	}

	// A mark wakes a channel that waits.
	woken := make(chan error)
	go func() { woken <- second.Marked(t.Context()) }()
	synctest.Wait()
	registry.Mark(bo, backend.SyncDevices)
	registry.Mark(ana, backend.SyncDevices)
	if err := <-woken; err != nil {
		t.Fatal(err)
	}
	if marked := second.Take(); !slices.Equal(marked.Parts, []backend.SyncPart{backend.SyncDevices}) {
		t.Fatalf("took %+v", marked)
	}

	first.Take()
	first.Close()
	registry.Mark(ana, backend.SyncCloud)
	ended, cancel := context.WithCancel(t.Context())
	cancel()
	if err := first.Marked(ended); !errors.Is(err, context.Canceled) {
		t.Fatalf("a closed channel was marked: %v", err)
	}
	if marked := second.Take(); !slices.Equal(marked.Parts, []backend.SyncPart{backend.SyncCloud}) {
		t.Fatalf("took %+v", marked)
	}
}
