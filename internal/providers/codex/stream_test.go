package codex_test

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"slices"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/codex"
)

func TestStreamEvents(t *testing.T) {
	v, _, p := setup(t)
	v.RespondAt(responses, stream(
		`{"type":"response.output_item.added","item":{"type":"reasoning","id":"rs_1"}}`,
		`{"type":"response.reasoning_text.delta","delta":"think"}`,
		`{"type":"response.output_item.done","item":{"type":"reasoning",`+
			`"id":"rs_1","encrypted_content":"enc"}}`,
		`{"type":"response.output_text.delta","delta":"hello"}`,
		`{"type":"response.output_item.done",`+
			`"item":{"type":"function_call","id":"fc_1","call_id":"call_1",`+
			`"name":"shell_exec","arguments":"{\"script\":\"pwd\"}"}}`,
		`{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":3}}}`,
	))
	events := run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())
	equal(
		t,
		events,
		[]provider.Event{
			&provider.ThinkingStart{},
			&provider.ThinkingDelta{Text: "think"},
			&provider.ThinkingSignature{Signature: `codex:{"type":"reasoning","id":"rs_1","encrypted_content":"enc"}`},
			&provider.TextDelta{Text: "hello"},
			&provider.ToolCall{
				ToolUseID: "call_1|fc_1",
				ToolName:  "shell_exec",
				Input:     json.RawMessage(`{"script":"pwd"}`),
			},
			&provider.Response{Usage: core.TokenUsage{InputTokens: 10, OutputTokens: 3}},
		},
	)
}

func TestStreamUsageLimit(t *testing.T) {
	v, _, p := setup(t)
	frame := `{"type":"error","error":{"type":"usage_limit_reached",` +
		`"message":"The usage limit has been reached","plan_type":"pro",` +
		`"resets_at":1790062659,"resets_in_seconds":321250},"status_code":429}`
	v.RespondAt(responses, stream(frame))
	f := failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest()))
	equal(t, *f.Code, provider.RateLimit)
	equal(t, f.Message, "The usage limit has been reached")
	equal(t, *f.RetryAfter, time.Duration(1790062659-1789740000)*time.Second)
	equal(t, f.Diagnostics.Source, core.FailureSource("stream"))
	equal(t, *f.Diagnostics.Upstream, frame)
	equal(t, *f.Diagnostics.ProviderCode, "usage_limit_reached")
}

func TestHTTPFailureRecord(t *testing.T) {
	v, _, p := setup(t)
	body := `{"error":{"code":"server_error","message":"backend failed"}}`
	a := answer(500, body)
	a.Headers = http.Header{"X-Request-Id": {"req-http-1"}, "Retry-After": {"2"}}
	v.RespondAt(responses, a)
	f := failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest()))
	equal(t, *f.Code, provider.Overloaded)
	equal(t, *f.RetryAfter, 2*time.Second)
	equal(t, *f.Diagnostics.ProviderRequestID, "req-http-1")
	equal(t, *f.Diagnostics.ProviderCode, "server_error")
	equal(t, f.Message, "Codex API request failed with HTTP 500: "+body)
	equal(t, f.Diagnostics.Source, core.FailureSource("http"))
	equal(t, *f.Diagnostics.HTTPStatus, uint16(500))
	record, _ := provider.ReadHTTPRecord(f.Diagnostics)
	equal(t, record.Body, body)
	var names []string
	for _, pair := range record.Headers {
		names = append(names, pair[0])
	}
	if !slices.Contains(names, "retry-after") || !slices.Contains(names, "x-request-id") || !slices.IsSorted(names) {
		t.Fatalf("headers: %v", names)
	}
	equal(t, len(v.Requests()), 1)
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
func TestHeaderTimeout(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		pool := poolWith(t, document(t, freshToken(t), "refresh-1", now))
		id := account
		config := codex.Config{Account: &id}
		config.Transport = codex.SSE
		config.HeaderTimeout = 50 * time.Millisecond
		client := &http.Client{
			Transport: roundTripFunc(
				func(r *http.Request) (*http.Response, error) { <-r.Context().Done(); return nil, r.Context().Err() },
			),
		}
		p, err := codex.New(config, pool, &provider.MemorySnapshots{}, client, providertest.FixedClock(now))
		if err != nil {
			t.Fatal(err)
		}
		f := failure(t, run(t.Context(), t, p, client, providertest.InferenceRequest()))
		equal(t, f.Diagnostics.Source, core.FailureSource("transport"))
		equal(t, f.Message, "Codex SSE response headers timed out after 50ms")
		equal(t, *f.Code, provider.Overloaded)
	})
}

func TestCancelStreamStopsDownload(t *testing.T) {
	v, _, p := setup(t)
	a := stream(`{"type":"response.output_text.delta","delta":"hel"}`)
	a.Ending = providertest.Open
	v.RespondAt(responses, a)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: v.Client()})
	if err != nil {
		t.Fatal(err)
	}
	var events []provider.Event
	for event := range runtime.Run(ctx, providertest.InferenceRequest()) {
		events = append(events, event)
		cancel()
	}
	equal(t, events, []provider.Event{&provider.TextDelta{Text: "hel"}})
	v.Disconnected(t.Context())
}

func TestFailureResetReader(t *testing.T) {
	_, _, p := setup(t)
	cases := []struct {
		text string
		want core.Timestamp
	}{
		{
			`{"type":"error","error":{"type":"usage_limit_reached",` +
				`"message":"The usage limit has been reached","plan_type":"pro",` +
				`"resets_at":1790062659,"resets_in_seconds":321250},"status_code":429}`,
			"2026-09-22T07:37:39.000Z",
		},
		{`{"error":{"resets_at":1790062659,"resets_in_seconds":1}}`, "2026-09-22T07:37:39.000Z"},
		{`{"type":"event","event":{"type":"error","error":{"resets_in_seconds":90}}}`, "2026-09-18T14:01:30.000Z"},
		{`{"type":"response.failed","response":{"error":{"resets_at":1790062659}}}`, "2026-09-22T07:37:39.000Z"},
		{`{"type":"error","error":{"resets_at":"soon"}}`, ""},
	}
	for _, test := range cases {
		d := core.ProviderErrorDiagnostics{Source: core.FailureSource("stream"), Upstream: &test.text}
		got := p.ReadFailure(&d, now).RetryAt
		if test.want == "" {
			equal(t, got, (*core.Timestamp)(nil))
		} else {
			equal(t, *got, test.want)
		}
	}
	for _, test := range []struct {
		body string
		want core.Timestamp
	}{{`{"error":{"resets_at":1790062659}}`, "2026-09-22T07:37:39.000Z"}, {"busy", "2026-09-18T14:00:30.000Z"}} {
		record := provider.NewHTTPFailureRecord(429, nil, test.body)
		if test.body == "busy" {
			record = provider.NewHTTPFailureRecord(503, http.Header{"Retry-After": {"30"}}, test.body)
		}
		b, err := provider.JSONBody(record)
		if err != nil {
			t.Fatal(err)
		}
		text := string(b)
		d := core.ProviderErrorDiagnostics{Source: core.FailureSource("http"), Upstream: &text}
		equal(t, *p.ReadFailure(&d, now).RetryAt, test.want)
	}
}

// controlledResponse supplies local protocol responses for fake-time login scenarios.
func controlledResponse(status int, text string) *http.Response {
	return &http.Response{StatusCode: status, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(text))}
}
