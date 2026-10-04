package openaiapi_test

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/openaiapi"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const now types.Timestamp = "2026-09-18T14:00:00.000Z"

var wires = []types.WireAPI{types.WireAPIResponses, types.WireAPIChatCompletions}

// providerAt configures an OpenAI entry against the scripted vendor.
func providerAt(
	t *testing.T,
	vendor *providertest.MockVendor,
	base string,
	wire types.WireAPI,
	policy provider.VendorPolicy,
) *openaiapi.Provider {
	t.Helper()
	endpoint, err := url.Parse(vendor.URL(base))
	if err != nil {
		t.Fatal(err)
	}
	key, err := provider.NewSecret("sk-test")
	if err != nil {
		t.Fatal(err)
	}
	return openaiapi.New(
		openaiapi.Config{APIKey: key, BaseURL: endpoint, Wire: wire, Policy: policy},
		providertest.FixedClock(now),
	)
}

// runtimeAt creates and owns a session runtime for a scripted vendor.
func runtimeAt(
	t *testing.T,
	vendor *providertest.MockVendor,
	base string,
	wire types.WireAPI,
	policy provider.VendorPolicy,
) provider.Runtime {
	t.Helper()
	runtime, err := providerAt(t, vendor, base, wire, policy).Runtime(provider.RuntimeEnv{HTTP: vendor.Client()})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := runtime.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return runtime
}

// requestWith creates the transcript sent in a provider scenario.
func requestWith(items ...provider.InferenceItem) provider.InferenceRequest {
	return provider.InferenceRequest{
		SessionID: "session-1",
		TurnID:    "turn-1",
		RequestID: "request-1",
		ModelID:   "gpt-test",
		Items:     items,
	}
}

// user creates a text message in a provider transcript.
func user(text string) provider.InferenceItem {
	return &provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: text}}}
}

// bodyOf observes the actual request sent to the vendor.
func bodyOf(
	ctx context.Context,
	t *testing.T,
	wire types.WireAPI,
	policy provider.VendorPolicy,
	request provider.InferenceRequest,
) map[string]any {
	t.Helper()
	vendor := providertest.StartVendor(t)
	vendor.Respond(providertest.EventStream("data: [DONE]\n\n"))
	providertest.Run(ctx, t, runtimeAt(t, vendor, "/v1", wire, policy), request)
	requests := vendor.Requests()
	if len(requests) != 1 {
		t.Fatalf("requests: %d", len(requests))
	}
	value, ok := requests[0].JSON(t).(map[string]any)
	if !ok {
		t.Fatal("request is not an object")
	}
	return value
}

// assertJSON compares a vendor-visible value with the expected JSON.
func assertJSON(t *testing.T, got any, want string) {
	t.Helper()
	var expected any
	if err := json.Unmarshal([]byte(want), &expected); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(expected, got) {
		t.Fatalf("vendor JSON:\nwant %#v\ngot  %#v", expected, got)
	}
}

func TestBuiltinCatalog(t *testing.T) {
	vendor := providertest.StartVendor(t)
	for _, wire := range wires {
		p := providerAt(t, vendor, "/v1", wire, provider.VendorPolicy{})
		providertest.AssertBuiltinCatalog(t.Context(), t, p, vendor)
		if p.Quota() != nil || p.Accounts() != nil {
			t.Fatal("API key entry exposes subscription state")
		}
	}
}

func TestEndpointsAndAnswers(t *testing.T) {
	vendor := providertest.StartVendor(t)
	cases := []struct {
		wire       types.WireAPI
		base, path string
	}{
		{types.WireAPIResponses, "/v1/", "/v1/responses"},
		{types.WireAPIResponses, "/gateway/v1/responses", "/gateway/v1/responses"},
		{types.WireAPIChatCompletions, "/openai/v1", "/openai/v1/chat/completions"},
		{types.WireAPIChatCompletions, "/v1/chat/completions/", "/v1/chat/completions"},
	}
	for _, tc := range cases {
		text := "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n"
		usage := types.TokenUsage{}
		if tc.wire == types.WireAPIResponses {
			text = `data: {"type":"response.output_text.delta","delta":"hi"}` +
				"\n\ndata: {\"type\":\"response.completed\"," +
				"\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n"
			usage.InputTokens = 3
			usage.OutputTokens = 1
		}
		vendor.Respond(providertest.EventStream(text))
		runtime := runtimeAt(t, vendor, tc.base, tc.wire, provider.VendorPolicy{})
		fresh := runtime.Fresh()
		events := providertest.Run(t.Context(), t, fresh, requestWith(user("hello")))
		if err := fresh.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		want := []provider.Event{&provider.TextDelta{Text: "hi"}, &provider.Response{Usage: usage}}
		if !reflect.DeepEqual(want, events) {
			t.Fatalf("events: want %#v, got %#v", want, events)
		}
		if !reflect.DeepEqual(provider.OpenAIRequestLimits(), runtime.RequestLimits(types.Model{})) {
			t.Fatal("wrong request limits")
		}
	}
	for i, request := range vendor.Requests() {
		if request.Method != "POST" || request.URI != cases[i].path {
			t.Fatalf("request: %+v", request)
		}
		for name, value := range map[string]string{
			"authorization": "Bearer sk-test",
			"content-type":  "application/json",
			"accept":        "text/event-stream",
		} {
			if request.Header(name) != value {
				t.Errorf("%s = %q", name, request.Header(name))
			}
		}
	}
}

func TestCancelledBeforeRun(t *testing.T) {
	vendor := providertest.StartVendor(t)
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	events := providertest.Run(
		ctx,
		t,
		runtimeAt(t, vendor, "/v1", types.WireAPIResponses, provider.VendorPolicy{}),
		requestWith(user("hello")),
	)
	if len(events) != 0 || len(vendor.Requests()) != 0 {
		t.Fatal("cancelled run emitted or sent")
	}
}

func TestCancelledMidStream(t *testing.T) {
	for _, wire := range wires {
		t.Run(string(wire), func(t *testing.T) {
			vendor := providertest.StartVendor(t)
			first := "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hel\"}\n\n"
			if wire == types.WireAPIChatCompletions {
				first = "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n"
			}
			response := providertest.EventStream(first)
			response.Ending = providertest.Open
			vendor.Respond(response)
			runtime := runtimeAt(t, vendor, "/v1", wire, provider.VendorPolicy{})
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			reader := providertest.NewEventReader(
				ctx,
				t,
				func(ctx context.Context) provider.Run { return runtime.Run(ctx, requestWith(user("hello"))) },
			)
			event, ok := reader.NextEvent()
			if !ok || !reflect.DeepEqual(event, &provider.TextDelta{Text: "hel"}) {
				t.Fatalf("first event: %#v", event)
			}
			cancel()
			if event, ok := reader.NextEvent(); ok {
				t.Fatalf("event after cancellation: %#v", event)
			}
			vendor.Disconnected(t.Context())
		})
	}
}

func TestHTTPFailureRecordAndWait(t *testing.T) {
	body := `{"error":{"message":"Rate limit reached","type":"requests","code":"rate_limit_exceeded"}}`
	for _, wire := range wires {
		vendor := providertest.StartVendor(t)
		vendor.Respond(
			providertest.MockResponse{
				Status:  429,
				Headers: http.Header{"Retry-After": []string{"20"}},
				Chunks:  [][]byte{[]byte(body)},
			},
		)
		events := providertest.Run(
			t.Context(),
			t,
			runtimeAt(t, vendor, "/v1", wire, provider.VendorPolicy{}),
			requestWith(user("hello")),
		)
		if len(events) != 1 {
			t.Fatalf("events: %#v", events)
		}
		event, ok := events[0].(*provider.Error)
		if !ok {
			t.Fatalf("event: %#v", events[0])
		}
		failure := event.Failure
		if failure.Message != "OpenAI API request failed with HTTP 429: "+
			body || failure.Code == nil ||
			*failure.Code != provider.RateLimit ||
			failure.RetryAfter == nil ||
			*failure.RetryAfter != 20*time.Second {
			t.Fatalf("failure: %+v", failure)
		}
		if failure.Diagnostics == nil || failure.Diagnostics.HTTPStatus == nil ||
			*failure.Diagnostics.HTTPStatus != 429 {
			t.Fatalf("diagnostic status: %+v", failure.Diagnostics)
		}
		record, ok := provider.ReadHTTPRecord(failure.Diagnostics)
		if !ok || record.Status != 429 || record.Body != body {
			t.Fatalf("record: %+v", record)
		}
		facts := providerAt(t, vendor, "/v1", wire, provider.VendorPolicy{}).ReadFailure(failure.Diagnostics, now)
		if facts.RetryAt == nil || *facts.RetryAt != "2026-09-18T14:00:20.000Z" {
			t.Fatalf("facts: %+v", facts)
		}
	}
}

func TestNoAnswerIsOverloaded(t *testing.T) {
	vendor := providertest.StartVendor(t)
	runtime := runtimeAt(t, vendor, "/v1", types.WireAPIResponses, provider.VendorPolicy{})
	vendor.Close()
	events := providertest.Run(t.Context(), t, runtime, requestWith(user("hello")))
	if len(events) != 1 {
		t.Fatalf("events: %#v", events)
	}
	event, ok := events[0].(*provider.Error)
	if !ok {
		t.Fatalf("event: %#v", events[0])
	}
	failure := event.Failure
	if failure.Code == nil || *failure.Code != provider.Overloaded || failure.Diagnostics.Source != "transport" ||
		strings.Contains(failure.Message, strings.TrimPrefix(vendor.URL(""), "http://")) {
		t.Fatalf("failure: %+v", failure)
	}
}
