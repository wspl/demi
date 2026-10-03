package plugins_test

import (
	"context"
	"encoding/json"
	"errors"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// Cleanup scenarios synchronize with calls and worker events, never elapsed time.
func TestDisableCancelsCallsAndJoinsInstanceWork(t *testing.T) {
	started := make(chan struct{})
	workerStopped := make(chan struct{})
	owner, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() {
		<-owner.Done()
		close(workerStopped)
	}()
	t.Cleanup(func() {
		cancel()
		<-workerStopped
	})
	var closes atomic.Int32
	p := &fakePlugin{call: func(ctx context.Context, _ plugin.Request, _ plugin.Port) (plugin.Reply, error) {
		close(started)
		<-ctx.Done()
		return nil, ctx.Err()
	}, close: func() {
		cancel()
		<-workerStopped
		closes.Add(1)
	}}
	u, _ := userFixture(
		t,
		registry(t, &fakeFactory{
			manifest: manifest(t, "worker"),
			make:     func() plugin.Plugin { return p },
		}),
	)
	answered := make(chan error, 1)
	go func() {
		_, err := u.PageCall(t.Context(), plugins.PageCall{Plugin: "worker", Method: "user", Params: []byte(`{}`)})
		answered <- err
	}()
	<-started
	changed, err := u.Switch(t.Context(), "worker", false)
	if err != nil || !changed || closes.Load() != 1 {
		t.Fatalf("%v %v closed=%d", changed, err, closes.Load())
	}
	if err := <-answered; !errors.Is(err, context.Canceled) {
		t.Fatalf("call cancellation lost: %v", err)
	}
	if err := u.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if closes.Load() != 1 {
		t.Fatal("instance closed twice")
	}
}

func TestCloseCancelsEveryInstanceAndRetainedPort(t *testing.T) {
	firstEntered, secondEntered := make(chan struct{}), make(chan struct{})
	var closes atomic.Int32
	makeFactory := func(id string, entered, otherEntered chan struct{}) *fakeFactory {
		return &fakeFactory{manifest: manifest(t, id), make: func() plugin.Plugin {
			return &fakePlugin{
				call: func(ctx context.Context, _ plugin.Request, _ plugin.Port) (plugin.Reply, error) {
					close(entered)
					<-otherEntered
					<-ctx.Done()
					return nil, ctx.Err()
				}, close: func() {
					closes.Add(1)
				},
			}
		}}
	}
	u, _ := userFixture(
		t,
		registry(
			t,
			makeFactory("first", firstEntered, secondEntered),
			makeFactory("second", secondEntered, firstEntered),
		),
	)
	done := make(chan error, 2)
	for _, id := range []string{"first", "second"} {
		go func() {
			_, err := u.PageState(t.Context(), id)
			done <- err
		}()
	}
	<-firstEntered
	<-secondEntered
	if err := u.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
	}
	if closes.Load() != 2 {
		t.Fatalf("closed %d instances", closes.Load())
	}
	if err := u.Close(t.Context()); err != nil || closes.Load() != 2 {
		t.Fatal("close not idempotent")
	}
	if _, err := u.PageState(t.Context(), "first"); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}

	retained, _, port, _ := portFixture(t)
	if _, err := port.WriteValue(t.Context(), "after-reply", json.RawMessage(`{}`), nil); err != nil {
		t.Fatalf("retained port stopped at reply: %v", err)
	}
	if err := retained.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if _, err := port.Value(t.Context(), "after-reply"); !errors.Is(err, context.Canceled) {
		t.Fatalf("port survived instance: %v", err)
	}
}

func TestConcurrentSwitchAndStorageFailure(t *testing.T) {
	var closes atomic.Int32
	m := manifest(t, "switch")
	u, shard := userFixture(
		t,
		registry(
			t,
			&fakeFactory{
				manifest: m,
				make: func() plugin.Plugin {
					return &fakePlugin{close: func() {
						closes.Add(1)
					}}
				},
			},
		),
	)
	type outcome struct {
		changed bool
		err     error
	}
	start := make(chan struct{})
	done := make(chan outcome, 2)
	for range 2 {
		go func() {
			<-start
			changed, err := u.Switch(t.Context(), "switch", false)
			done <- outcome{changed, err}
		}()
	}
	close(start)
	changes := 0
	for range 2 {
		got := <-done
		if got.err != nil {
			t.Fatal(got.err)
		}
		if got.changed {
			changes++
		}
	}
	if changes != 1 || closes.Load() != 1 {
		t.Fatalf("changed %d closed %d", changes, closes.Load())
	}
	if err := shard.control.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	changed, err := u.Switch(t.Context(), "switch", true)
	var refused *plugins.SwitchError
	if changed || !errors.As(err, &refused) || !errors.Is(err, database.ErrClosed) {
		t.Fatalf("%v %v", changed, err)
	}
	entries, err := u.Entries(t.Context())
	if err != nil || entries[0].Enabled {
		t.Fatalf("failed commit changed enabled choice: %+v %v", entries, err)
	}
}

func TestRequestAndPortCancellation(t *testing.T) {
	u, shard, _, port := portFixture(t)
	shard.hosts = func(ctx context.Context, _ webapi.ConversationID) ([]plugin.ConversationHost, error) {
		<-ctx.Done()
		return nil, ctx.Err()
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if _, err := port.ConversationHosts(ctx); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	if err := u.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	var ended *host.PortError
	if _, err := port.Values(t.Context()); !errors.As(err, &ended) || ended.Kind != host.PortEnded {
		t.Fatal(err)
	}
}
