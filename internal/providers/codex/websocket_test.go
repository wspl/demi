package codex_test

import (
	"context"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/codex"
	"github.com/wspl/demi/internal/providers/codex/codextest"
)

const wsDone = `{"type":"response.completed",` +
	`"response":{"usage":{"input_tokens":1,"output_tokens":1}}}`

func socketProvider(
	t *testing.T,
	socket *codextest.FakeWebSocket,
	authURL string,
	mode codex.TransportMode,
	idle *time.Duration,
	clock core.Clock,
) *codex.Provider {
	t.Helper()
	pool := poolWith(t, document(t, freshToken(t), "refresh-1", now))
	id := account
	config := codex.NewConfig(&id)
	config.BackendURL = socket.BackendURL()
	config.AuthURL = authURL
	config.Transport = mode
	config.StreamIdleTimeout = idle
	p, err := codex.New(config, pool, &provider.MemorySnapshots{}, socket.Client(), clock)
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func TestWebSocketRequest(t *testing.T) {
	socket := codextest.Start(
		t,
		[]codextest.Script{
			{Steps: []codextest.Step{{Text: `{"type":"response.output_text.delta","delta":"ws"}`}, {Text: wsDone}}},
		},
		"",
	)
	p := socketProvider(t, socket, "http://codex.test", codex.WebSocket, nil, providertest.FixedClock(now))
	r := providertest.InferenceRequest()
	r.ModelID = "gpt-5.4"
	events := run(t.Context(), t, p, socket.Client(), r)
	equal(
		t,
		events,
		[]provider.Event{
			&provider.TextDelta{Text: "ws"},
			&provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1}},
		},
	)
	connection := socket.Connections()[0]
	equal(t, connection.Headers.Get("Openai-Beta"), "responses_websockets=2026-02-06")
	equal(t, connection.Headers.Get("Authorization"), "Bearer "+freshToken(t))
	equal(t, connection.Headers.Get("Chatgpt-Account-Id"), "acct-1")
	equal(t, connection.Headers.Values("Accept"), []string(nil))
	equal(t, connection.Headers.Values("Content-Type"), []string(nil))
	equal(t, len(connection.Received), 1)
	body := jsonObject(t, []byte(connection.Received[0]))
	equal(t, body["type"], "response.create")
	equal(t, body["model"], "gpt-5.4")
	equal(t, body["stream"], true)
	reason, err := socket.CloseReason(t.Context(), 0)
	if err != nil {
		t.Fatal(err)
	}
	equal(t, reason, "response_done")
	equal(t, *p.Quota().Latest().Windows[0].UsedPercent, float64(12))
	// Both transports carry identical field bytes after the WebSocket type.
	httpRequest := sent(t, r)
	equal(t, connection.Received[0], `{"type":"response.create",`+string(httpRequest.Body[1:]))
}

func TestWebSocketEnvelopes(t *testing.T) {
	socket := codextest.Start(
		t,
		[]codextest.Script{
			{
				Steps: []codextest.Step{
					{Text: `{"type":"event","event":{"type":"response.output_text.delta","delta":"hi"}}`},
					{Text: `{"type":"response.done","response":{"usage":{"input_tokens":1,"output_tokens":1}}}`},
				},
			},
		},
		"",
	)
	p := socketProvider(t, socket, "http://codex.test", codex.WebSocket, nil, providertest.FixedClock(now))
	equal(
		t,
		run(t.Context(), t, p, socket.Client(), providertest.InferenceRequest()),
		[]provider.Event{
			&provider.TextDelta{Text: "hi"},
			&provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1}},
		},
	)
}

func TestWebSocketEarlyCloseFallback(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(responses, completed())
	socket := codextest.Start(t, []codextest.Script{{Steps: []codextest.Step{{Kind: codextest.Close}}}}, v.URL(""))
	p := socketProvider(t, socket, v.URL(""), codex.Auto, nil, providertest.FixedClock(now))
	equal(
		t,
		run(t.Context(), t, p, socket.Client(), providertest.InferenceRequest()),
		[]provider.Event{&provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1}}},
	)
	equal(t, len(v.Requests()), 1)
	equal(t, v.Requests()[0].Header("Accept"), "text/event-stream")
}

func TestWebSocketUnreachableCooldown(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(responses, completed())
	v.RespondAt(responses, completed())
	socket := codextest.Start(
		t,
		[]codextest.Script{{Handshake: codextest.Disconnect}, {Steps: []codextest.Step{{Text: wsDone}}}},
		v.URL(""),
	)
	clock := providertest.NewManualClock(now)
	p := socketProvider(t, socket, v.URL(""), codex.Auto, nil, clock)
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: socket.Client()})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := runtime.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	other := runtime.Fresh()
	defer func() {
		if err := other.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	for i := range 3 {
		if i == 1 {
			if err := clock.Advance(599 * time.Second); err != nil {
				t.Fatal(err)
			}
		}
		if i == 2 {
			if err := clock.Advance(time.Second); err != nil {
				t.Fatal(err)
			}
		}
		selected := runtime
		if i > 0 {
			selected = other
		}
		equal(
			t,
			providertest.Run(t.Context(), t, selected, providertest.InferenceRequest()),
			[]provider.Event{&provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1}}},
		)
		expected := 1
		if i == 2 {
			expected = 2
		}
		equal(t, len(socket.Connections()), expected)
		equal(t, len(v.Requests()), min(i+1, 2))
	}
	equal(t, len(v.Requests()), 2)
}

func TestWebSocketFailureAfterEvent(t *testing.T) {
	v := providertest.StartVendor(t)
	socket := codextest.Start(
		t,
		[]codextest.Script{
			{
				Steps: []codextest.Step{
					{Text: `{"type":"response.output_text.delta","delta":"ws"}`},
					{Kind: codextest.Drop},
				},
			},
		},
		v.URL(""),
	)
	p := socketProvider(t, socket, v.URL(""), codex.Auto, nil, providertest.FixedClock(now))
	events := run(t.Context(), t, p, socket.Client(), providertest.InferenceRequest())
	equal(t, len(events), 2)
	equal(t, events[0], &provider.TextDelta{Text: "ws"})
	f := failure(t, events[1:])
	equal(t, *f.Code, provider.Overloaded)
	if !strings.HasPrefix(f.Message, "Codex WebSocket failed") {
		t.Fatal(f.Message)
	}
	equal(t, len(v.Requests()), 0)
}

func TestWebSocketHandshakeRefresh(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt("/oauth/token", answer(200, `{"access_token":"new-access","refresh_token":"refresh-2"}`))
	socket := codextest.Start(
		t,
		[]codextest.Script{{Handshake: codextest.Reject, Status: 401}, {Steps: []codextest.Step{{Text: wsDone}}}},
		v.URL(""),
	)
	// Route login through the fixture as well, which forwards ordinary HTTP to the vendor.
	p := socketProvider(t, socket, "http://codex.test", codex.WebSocket, nil, providertest.FixedClock(now))
	events := run(t.Context(), t, p, socket.Client(), providertest.InferenceRequest())
	equal(t, events, []provider.Event{&provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1}}})
	equal(t, socket.Connections()[1].Headers.Get("Authorization"), "Bearer new-access")
	equal(t, len(v.Requests()), 1)
}

func TestWebSocketIdleClose(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		socket := codextest.Start(
			t,
			[]codextest.Script{{Steps: []codextest.Step{{Text: `{"type":"response.output_text.delta","delta":"ws"}`}}}},
			"",
		)
		idle := 100 * time.Millisecond
		p := socketProvider(t, socket, "http://codex.test", codex.WebSocket, &idle, providertest.FixedClock(now))
		events := run(t.Context(), t, p, socket.Client(), providertest.InferenceRequest())
		equal(t, len(events), 2)
		if _, ok := events[0].(*provider.TextDelta); !ok {
			t.Fatalf("first event: %#v", events[0])
		}
		f := failure(t, events[1:])
		equal(t, f.Message, "Codex WebSocket stream idled for 100ms")
		equal(t, *f.Code, provider.Overloaded)
		reason, err := socket.CloseReason(t.Context(), 0)
		if err != nil {
			t.Fatal(err)
		}
		equal(t, reason, "idle_timeout")
	})
}

func TestWebSocketCancelClose(t *testing.T) {
	socket := codextest.Start(
		t,
		[]codextest.Script{{Steps: []codextest.Step{{Text: `{"type":"response.output_text.delta","delta":"hel"}`}}}},
		"",
	)
	p := socketProvider(t, socket, "http://codex.test", codex.WebSocket, nil, providertest.FixedClock(now))
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: socket.Client()})
	if err != nil {
		t.Fatal(err)
	}
	var events []provider.Event
	for event := range runtime.Run(ctx, providertest.InferenceRequest()) {
		events = append(events, event)
		cancel()
	}
	equal(t, events, []provider.Event{&provider.TextDelta{Text: "hel"}})
	reason, err := socket.CloseReason(t.Context(), 0)
	if err != nil {
		t.Fatal(err)
	}
	equal(t, reason, "aborted")
}

func TestWebSocketRefusalKeepsWholeBody(t *testing.T) {
	v, pool, _ := setup(t)
	body := strings.Repeat("backend unavailable ", 150)
	v.RespondAt(responses, answer(503, body))
	p := configured(t, v, pool, func(c *codex.Config) { c.Transport = codex.WebSocket })
	f := failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest()))
	equal(t, provider.ReadHTTPRecord(f.Diagnostics).Body, body)
	equal(t, len(v.Requests()), 1)
}
