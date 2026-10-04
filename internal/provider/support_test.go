package provider_test

import (
	"context"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestScriptedRuntimeSharesTurnsAndOwnsCancellation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		runtime := providertest.NewScriptedRuntime(t,
			providertest.Events(providertest.Text("first"), providertest.Response(2, 3)),
			providertest.Respond(func(request provider.InferenceRequest) []provider.Event {
				return []provider.Event{providertest.Text(request.RequestID)}
			}),
			providertest.Pending(),
		)
		request := providertest.InferenceRequest()
		reader := providertest.NewEventReader(
			t.Context(),
			t,
			func(ctx context.Context) provider.Run {
				return runtime.Run(ctx, request)
			},
		)
		event, ok := reader.NextEvent()
		requireEqual(t, ok, true)
		requireEqual(t, event, providertest.Text("first"))
		time.Sleep(11 * time.Second) // Virtual time between reads must not spend the next read's hang guard.
		event, ok = reader.NextEvent()
		requireEqual(t, ok, true)
		requireEqual(t, event, providertest.Response(2, 3))
		reader.Close()
		fresh := runtime.Fresh()
		requireEqual(
			t,
			providertest.Run(t.Context(), t, fresh, request),
			[]provider.Event{providertest.Text(request.RequestID)},
		)
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		stopped := make(chan []provider.Event, 1)
		go func() {
			stopped <- providertest.Run(ctx, t, runtime, request)
		}()
		synctest.Wait()
		cancel()
		requireEqual(t, <-stopped, []provider.Event{})
		requireEqual(t, runtime.Remaining(), 0)
		requireEqual(t, len(runtime.Requests()), 3)
		if err := fresh.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		if err := runtime.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		requireEqual(t, runtime.Closes(), 2)
	})
}
