package google_test

import (
	"context"
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

// streamEvents observes mapped events from a scripted Gemini stream.
func streamEvents(t *testing.T, payloads ...string) []provider.Event {
	t.Helper()
	var frames strings.Builder
	for _, payload := range payloads {
		frames.WriteString("data: " + payload + "\n\n")
	}
	return responseEvents(t, providertest.EventStream(frames.String()))
}

// responseEvents runs a request against one scripted vendor response.
func responseEvents(t *testing.T, response providertest.MockResponse) []provider.Event {
	t.Helper()
	v := providertest.StartVendor(t)
	v.Respond(response)
	return providertest.Run(t.Context(), t, runtimeAt(t, v, "/v1beta"), providertest.InferenceRequest())
}

// onlyFailure requires exactly one terminal failure from the run.
func onlyFailure(t *testing.T, events []provider.Event) provider.Failure {
	t.Helper()
	if len(events) != 1 {
		t.Fatalf("expected one failure, got %#v", events)
	}
	event, ok := events[0].(*provider.Error)
	if !ok {
		t.Fatalf("expected error, got %#v", events[0])
	}
	return event.Failure
}

func TestThoughtsAndCallSignature(t *testing.T) {
	events := streamEvents(
		t,
		`{"candidates":[{"content":{"parts":[{"text":"weighing options","thought":true}]}}]}`,
		`{"candidates":[{"content":{"parts":[{`+
			`"functionCall":{"name":"shell_exec","args":{"command":"ls"},"id":"c1"},`+
			`"thoughtSignature":"sig-1"}]}}]}`,
		`{"candidates":[{"content":{"parts":[{"text":"done"}]}}]}`,
		`{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":4,`+
			`"thoughtsTokenCount":20,"cachedContentTokenCount":3}}`,
	)
	want := []provider.Event{
		&provider.ThinkingStart{}, &provider.ThinkingDelta{Text: "weighing options"},
		&provider.ThinkingSignature{Signature: "google:sig-1"},
		&provider.ToolCall{ToolUseID: "c1", ToolName: "shell_exec", Input: json.RawMessage(`{"command":"ls"}`)},
		&provider.TextDelta{Text: "done"},
		&provider.Response{Usage: types.TokenUsage{InputTokens: 7, OutputTokens: 24, CacheReadTokens: 3}},
	}
	if !reflect.DeepEqual(want, events) {
		t.Fatalf("got %#v; want %#v", events, want)
	}
}

func TestSignedCallOpensThinking(t *testing.T) {
	events := streamEvents(
		t,
		`{"candidates":[{"content":{"parts":[{`+
			`"functionCall":{"name":"look","id":"c1"},"thoughtSignature":"sig-1"}]}}]}`,
		`{"candidates":[{"content":{"parts":[{"text":"plan",`+
			`"thought":true},{"text":"","thoughtSignature":"sig-2"},`+
			`{"text":"answer"}]}}]}`,
	)
	want := []provider.Event{
		&provider.ThinkingStart{},
		&provider.ThinkingSignature{Signature: "google:sig-1"},
		&provider.ToolCall{ToolUseID: "c1", ToolName: "look", Input: json.RawMessage(`{}`)},
		&provider.ThinkingStart{},
		&provider.ThinkingDelta{Text: "plan"},
		&provider.ThinkingSignature{Signature: "google:sig-2"},
		&provider.TextDelta{Text: "answer"},
		&provider.Response{},
	}
	if !reflect.DeepEqual(want, events) {
		t.Fatalf("got %#v; want %#v", events, want)
	}
}

func TestMissingCallIDsAreUnique(t *testing.T) {
	call := `{"candidates":[{"content":{"parts":[{"functionCall":{"name":"shell_exec","args":{}}}]}}]}`
	all := append(streamEvents(t, call, call), streamEvents(t, call)...)
	ids := make(map[string]bool)
	for _, event := range all {
		if call, ok := event.(*provider.ToolCall); ok {
			if !strings.HasPrefix(call.ToolUseID, "shell_exec_") || ids[call.ToolUseID] {
				t.Fatalf("invalid or repeated ID: %s", call.ToolUseID)
			}
			ids[call.ToolUseID] = true
		}
	}
	if len(ids) != 3 {
		t.Fatalf("got %d unique calls", len(ids))
	}
}

func TestStreamErrorRecord(t *testing.T) {
	chunk := `{"error":{"status":"RESOURCE_EXHAUSTED","message":"quota exceeded"}}`
	failure := onlyFailure(t, streamEvents(t, chunk, `{"candidates":[{"content":{"parts":[{"text":"never read"}]}}]}`))
	if failure.Message != "quota exceeded" || failure.Code == nil || *failure.Code != provider.RateLimit {
		t.Fatalf("failure: %+v", failure)
	}
	d := failure.Diagnostics
	if d == nil || d.Source != "stream" || d.ProviderCode == nil || *d.ProviderCode != "RESOURCE_EXHAUSTED" ||
		d.Upstream == nil ||
		*d.Upstream != chunk {
		t.Fatalf("diagnostics: %+v", d)
	}
}

func TestMalformedChunkNamesField(t *testing.T) {
	for _, tc := range []struct{ chunk, field string }{
		{`{"candidates":{"content":{"parts":[{"text":"hello"}]}}}`, "candidates"},
		{`{"usageMetadata":{"promptTokenCount":"10"}}`, "promptTokenCount"},
		{`{"candidates":[{"content":{"parts":[{"functionCall":{"name":""}}]}}]}`, "name"},
	} {
		t.Run(tc.field, func(t *testing.T) {
			failure := onlyFailure(t, streamEvents(t, tc.chunk))
			if failure.Code != nil || !strings.Contains(failure.Message, tc.field) || failure.Diagnostics == nil ||
				failure.Diagnostics.Upstream == nil ||
				*failure.Diagnostics.Upstream != tc.chunk {
				t.Fatalf("failure: %+v", failure)
			}
		})
	}
}

func TestRefusedRequestRecord(t *testing.T) {
	body := `{"error":{"code":429,"message":"Resource has been exhausted",` +
		`"status":"RESOURCE_EXHAUSTED"}}`
	failure := onlyFailure(t, responseEvents(t, providertest.MockResponse{Status: 429, Chunks: [][]byte{[]byte(body)}}))
	if failure.Message != "Google API request failed with HTTP 429: "+
		body || failure.Code == nil ||
		*failure.Code != provider.RateLimit {
		t.Fatalf("failure: %+v", failure)
	}
	record, ok := provider.ReadHTTPRecord(failure.Diagnostics)
	if !ok || record.Body != body || record.Status != 429 {
		t.Fatalf("record: %+v", record)
	}
}

func TestCancellationClosesStream(t *testing.T) {
	v := providertest.StartVendor(t)
	response := providertest.EventStream("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hel\"}]}}]}\n\n")
	response.Ending = providertest.Open
	v.Respond(response)
	r := runtimeAt(t, v, "/v1beta")
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	reader := providertest.NewEventReader(
		ctx,
		t,
		func(ctx context.Context) provider.Run { return r.Run(ctx, providertest.InferenceRequest()) },
	)
	event, ok := reader.NextEvent()
	if !ok || !reflect.DeepEqual(event, &provider.TextDelta{Text: "hel"}) {
		t.Fatalf("first event: %#v", event)
	}
	cancel()
	if event, ok := reader.NextEvent(); ok {
		t.Fatalf("event after cancel: %#v", event)
	}
	v.Disconnected(t.Context())

	untouched := providertest.StartVendor(t)
	if events := providertest.Run(
		ctx,
		t,
		runtimeAt(t, untouched, "/v1beta"),
		providertest.InferenceRequest(),
	); len(
		events,
	) != 0 {
		t.Fatalf("pre-cancelled events: %#v", events)
	}
	if len(untouched.Requests()) != 0 {
		t.Fatal("pre-cancelled run reached vendor")
	}
}

func TestToolArgumentsKeepVendorJSONMeaning(t *testing.T) {
	for _, tc := range []struct{ name, args, want string }{
		{"object order and duplicate", `{"z":1,"a":2,"z":3}`, `{"z":3,"a":2}`},
		{"string", `"not an object"`, `"not an object"`},
		{"null", "null", `{}`},
		{"float", `{"value":1.0}`, `{"value":1.0}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			events := streamEvents(
				t,
				`{"candidates":[{"content":{"parts":[{`+
					`"functionCall":{"name":"tool","id":"c","args":`+
					tc.args+
					`}}]}}]}`,
			)
			if len(events) != 2 {
				t.Fatalf("events: %#v", events)
			}
			call, ok := events[0].(*provider.ToolCall)
			if !ok || string(call.Input) != tc.want {
				t.Fatalf("call: %#v", events[0])
			}
		})
	}
	failure := onlyFailure(
		t,
		streamEvents(
			t,
			`{"candidates":[{"content":{"parts":[{`+
				`"functionCall":{"name":"tool","args":{"bad":"\ud800"}}}]}}]}`,
		),
	)
	if failure.Code != nil || !strings.Contains(failure.Message, "args") {
		t.Fatalf("malformed arguments: %+v", failure)
	}
}

func TestStreamToleranceAndLatestUsage(t *testing.T) {
	events := streamEvents(
		t,
		`{"unknown":{"future":true},"candidates":null,`+
			`"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":50}}`,
		`{"candidates":[{}, {"content":{"parts":[{"futurePart":true},`+
			`{"text":"","thought":false},{"thoughtSignature":"ignored"}]}}],`+
			`"usageMetadata":{"promptTokenCount":2,"cachedContentTokenCount":5}}`,
	)
	want := []provider.Event{&provider.Response{Usage: types.TokenUsage{CacheReadTokens: 5}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
	failure := onlyFailure(t, streamEvents(t, `{"error":{"message":17,"status":[]}}`))
	if failure.Message != "Google API stream error" || failure.Code != nil {
		t.Fatalf("reported fields: %+v", failure)
	}
}

func TestEarlyConsumerExitClosesConnection(t *testing.T) {
	v := providertest.StartVendor(t)
	response := providertest.EventStream(
		"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello\"}]}}]}\n\n",
	)
	response.Ending = providertest.Open
	v.Respond(response)
	for event := range runtimeAt(t, v, "/v1beta").Run(t.Context(), providertest.InferenceRequest()) {
		if !reflect.DeepEqual(event, &provider.TextDelta{Text: "hello"}) {
			t.Fatalf("event: %#v", event)
		}
		break
	}
	v.Disconnected(t.Context())
}

func TestBrokenStreamFailsInsteadOfCompleting(t *testing.T) {
	failure := onlyFailure(t, responseEvents(t, providertest.MockResponse{Status: 200, Ending: providertest.Broken}))
	if failure.Code == nil || *failure.Code != provider.Overloaded {
		t.Fatalf("failure: %+v", failure)
	}
}
