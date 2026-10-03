package provider_test

import (
	"context"
	"encoding/json"
	"io"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"go.uber.org/goleak"
)

// All tests use fixtures or loopback servers, never a real vendor. The package's
// regular suite is budgeted at 10 seconds; time-based scenarios use synctest.
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const now = core.Timestamp("2026-09-18T14:00:00.000Z")

// requireEqual compares observable provider results with a useful failure location.
func requireEqual(t *testing.T, got, want any) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v; want %#v", got, want)
	}
}

// encoded creates vendor test input with the same JSON encoder as production.
func encoded(t *testing.T, value any) string {
	t.Helper()
	data, err := contract.EncodeJSON(value)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// streamBody builds a response from exact vendor frames.
func streamBody(frames ...string) io.ReadCloser {
	var text strings.Builder
	for _, frame := range frames {
		text.WriteString("data: " + frame + "\n\n")
	}
	return io.NopCloser(strings.NewReader(text.String()))
}

// mapped runs a recorded Responses or Chat Completions stream under a hang guard.
func mapped(t *testing.T, chat bool, frames ...string) []provider.Event {
	t.Helper()
	return mappedWith(t.Context(), t, chat, provider.ReadHTTPFailure, frames...)
}

func mappedWith(
	ctx context.Context,
	t *testing.T,
	chat bool,
	reader provider.FailureReader,
	frames ...string,
) []provider.Event {
	t.Helper()
	return providertest.AllEvents(ctx, t, func(ctx context.Context) provider.Run {
		label := "Codex"
		if chat {
			label = "Grok Build"
		}
		vendor := provider.Vendor{Label: label, Reader: reader, Clock: providertest.FixedClock(now)}
		if chat {
			return provider.MapChatSSE(ctx, streamBody(frames...), vendor)
		}
		return provider.MapResponsesEvents(
			ctx,
			provider.ResponsesSSEEvents(ctx, streamBody(frames...), label),
			vendor,
			"codex:",
		)
	})
}

// failureOf requires a stream to terminate with exactly one failure event.
func failureOf(t *testing.T, events []provider.Event) provider.Failure {
	t.Helper()
	if len(events) != 1 {
		t.Fatalf("expected one failure event, got %#v", events)
	}
	event, ok := events[0].(*provider.Error)
	if !ok {
		t.Fatalf("expected failure, got %#v", events[0])
	}
	return event.Failure
}

// jsonValue compares wire semantics while retaining whole numbers.
func jsonValue(t *testing.T, text string) any {
	t.Helper()
	var value any
	decoder := json.NewDecoder(strings.NewReader(text))
	decoder.UseNumber()
	if err := decoder.Decode(&value); err != nil {
		t.Fatal(err)
	}
	return value
}
