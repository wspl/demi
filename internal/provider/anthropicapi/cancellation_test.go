package anthropicapi_test

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestCancelledRequestSendsNothing(t *testing.T) {
	v := providertest.StartVendor(t)
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	events := providertest.Run(ctx, t, testRuntime(t, v, provider.VendorPolicy{}), providertest.InferenceRequest())
	if len(events) != 0 || len(v.Requests()) != 0 {
		t.Fatal(events, v.Requests())
	}
}

// openReader starts a response that stays open after its first text delta.
func openReader(ctx context.Context, t *testing.T) (*providertest.MockVendor, *providertest.EventReader) {
	t.Helper()
	v := providertest.StartVendor(t)
	response := recorded(
		`{"type":"message_start","message":{"usage":{"input_tokens":3}}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hel"}}`,
	)
	response.Ending = providertest.Open
	v.Respond(response)
	r := testRuntime(t, v, provider.VendorPolicy{})
	reader := providertest.NewEventReader(
		ctx,
		t,
		func(ctx context.Context) provider.Run {
			return r.Run(ctx, providertest.InferenceRequest())
		},
	)
	event, ok := reader.NextEvent()
	if !ok {
		t.Fatal("no text event")
	}
	equalEvents(t, []provider.Event{event}, []provider.Event{&provider.TextDelta{Text: "hel"}})
	return v, reader
}

func TestCancelMidStream(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	v, reader := openReader(ctx, t)
	cancel()
	if event, ok := reader.NextEvent(); ok {
		t.Fatal(event)
	}
	v.Disconnected(t.Context())
}

func TestDropMidStream(t *testing.T) {
	v, reader := openReader(t.Context(), t)
	reader.Close()
	v.Disconnected(t.Context())
}
