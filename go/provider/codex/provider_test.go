package codex_test

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"net/http"
	"net/http/httptest"
	"net/url"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/codex"
	"github.com/wspl/demi/go/provider/providertest"
)

// fixture provides a stored subscription against the scenario's vendor.
func fixture(t *testing.T, endpoint string, mode codex.TransportMode) (*codex.Provider, *provider.MemoryCredentialPool) {
	t.Helper()
	token, err := provider.NewSecret("access")
	if err != nil {
		t.Fatal(err)
	}
	document := codex.SecretDocument{AccessToken: token, RefreshToken: token, IDToken: token, AccountID: "account", LastRefresh: core.UnixEpoch}
	raw, err := document.JSON()
	if err != nil {
		t.Fatal(err)
	}
	pool := provider.NewMemoryCredentialPool()
	if err := pool.Write(t.Context(), provider.AccountMeta{ID: "one", Label: "account"}, string(raw)); err != nil {
		t.Fatal(err)
	}
	config := codex.NewConfig(new("one"))
	base, err := url.Parse(endpoint)
	if err != nil {
		t.Fatal(err)
	}
	config.BackendURL = *base
	config.AuthURL = *base
	config.Transport = mode
	p, err := codex.New(config, pool, &provider.MemorySnapshots{}, http.DefaultClient, providertest.FixedClock{Time: core.UnixEpoch})
	if err != nil {
		t.Fatal(err)
	}
	return p, pool
}
func run(t *testing.T, p *codex.Provider) []provider.ProviderEvent {
	t.Helper()
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: http.DefaultClient})
	if err != nil {
		t.Fatal(err)
	}
	return providertest.Run(t, runtime, providertest.InferenceRequest())
}

// Cost: loopback requests only; two-minute contexts guard missing events.
func TestSSERefreshAndRequest(t *testing.T) {
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Status: 401},
		providertest.MockResponse{Chunks: []string{`{"access_token":"renewed"}`}},
		providertest.MockResponse{Chunks: []string{providertest.SSEBody(jsontext.Value(`{"type":"response.output_text.delta","delta":"hello"}`), jsontext.Value(`{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":3,"input_tokens_details":{"cached_tokens":4}}}}`))}},
	)
	p, pool := fixture(t, vendor.Server.URL, codex.TransportSSE)
	events := run(t, p)
	want := []provider.ProviderEvent{provider.TextDelta{Text: "hello"}, provider.Response{Usage: core.TokenUsage{InputTokens: 6, OutputTokens: 3, CacheReadTokens: 4}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
	requests := vendor.Requests()
	if len(requests) != 3 || requests[1].URI != "/oauth/token" || requests[2].Headers.Get("Authorization") != "Bearer renewed" {
		t.Fatalf("requests: %#v", requests)
	}
	if requests[0].URI != "/codex/responses" || requests[0].Headers.Get("Chatgpt-Account-Id") != "account" || requests[0].Headers.Get("Session-Id") != "session-1" {
		t.Fatal("subscription headers or endpoint missing")
	}
	var body map[string]jsontext.Value
	if err := json.Unmarshal(requests[2].Body, &body); err != nil {
		t.Fatal(err)
	}
	for key, want := range map[string]string{"store": "false", "stream": "true", "instructions": `""`, "tools": "[]", "tool_choice": `"auto"`, "text": `{"verbosity":"low"}`} {
		if string(body[key]) != want {
			t.Fatalf("%s: %s", key, body[key])
		}
	}
	if _, ok := body["max_output_tokens"]; ok {
		t.Fatal("Codex cannot accept an output limit")
	}
	if !strings.Contains(pool.Entries()[0].Secret, "renewed") {
		t.Fatal("refresh not persisted")
	}
}

func TestWebSocketEnvelopesAndFallbackWindow(t *testing.T) {
	for _, scenario := range []string{"envelope", "before-first", "after-first"} {
		t.Run(scenario, func(t *testing.T) {
			var httpCalls atomic.Int32
			closed := make(chan string, 1)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Upgrade") == "" {
					httpCalls.Add(1)
					w.Header().Set("Content-Type", "text/event-stream")
					_, _ = w.Write([]byte("data: {\"type\":\"response.output_text.delta\",\"delta\":\"fallback\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n"))
					return
				}
				conn, err := websocket.Accept(w, r, nil)
				if err != nil {
					return
				}
				defer conn.CloseNow()
				ctx, cancel := context.WithTimeout(r.Context(), 2*time.Minute)
				defer cancel()
				_, body, err := conn.Read(ctx)
				if err != nil {
					t.Error(err)
					return
				}
				var request map[string]jsontext.Value
				if err := json.Unmarshal(body, &request); err != nil || string(request["type"]) != `"response.create"` {
					t.Error("missing response.create request")
				}
				if scenario != "before-first" {
					if err := conn.Write(ctx, websocket.MessageBinary, []byte(`{"event":{"type":"response.output_text.delta","delta":"socket"}}`)); err != nil {
						return
					}
				}
				terminal := `{"type":"response.done","response":{"usage":{"output_tokens":2}}}`
				if scenario != "envelope" {
					terminal = `{"type":"response.output_text.delta","delta":12}`
				}
				if err := conn.Write(ctx, websocket.MessageText, []byte(terminal)); err != nil {
					return
				}
				_, _, err = conn.Read(ctx)
				if err != nil {
					closed <- err.Error()
				}
			}))
			defer server.Close()
			p, _ := fixture(t, server.URL, codex.TransportAuto)
			events := run(t, p)
			switch scenario {
			case "envelope":
				if !reflect.DeepEqual(events, []provider.ProviderEvent{provider.TextDelta{Text: "socket"}, provider.Response{Usage: core.TokenUsage{OutputTokens: 2}}}) {
					t.Fatalf("events %#v", events)
				}
			case "before-first":
				if len(events) != 2 || events[0] != (provider.TextDelta{Text: "fallback"}) || httpCalls.Load() != 1 {
					t.Fatalf("fallback: %#v, HTTP %d", events, httpCalls.Load())
				}
			case "after-first":
				if len(events) != 2 {
					t.Fatalf("events %#v", events)
				}
				if _, ok := events[1].(provider.FailureEvent); !ok || httpCalls.Load() != 0 {
					t.Fatalf("late failure: %#v", events)
				}
			}
			ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
			defer cancel()
			select {
			case reason := <-closed:
				if scenario == "envelope" && !strings.Contains(reason, "response_done") {
					t.Fatalf("close %s", reason)
				}
			case <-ctx.Done():
				t.Fatal("websocket did not close")
			}
		})
	}
}

func TestWebSocketRefusalRetainsWholeRecord(t *testing.T) {
	body := `{"error":{"code":"usage_limit_reached","message":"` + strings.Repeat("x", 4096) + `"},"request_id":"body-id"}`
	vendor := providertest.NewMockVendor(t, providertest.MockResponse{Status: 429, Headers: http.Header{"X-Request-Id": {"header-id"}}, Chunks: []string{body}})
	p, _ := fixture(t, vendor.Server.URL, codex.TransportWebSocket)
	events := run(t, p)
	if len(events) != 1 {
		t.Fatalf("events %#v", events)
	}
	failure, ok := events[0].(provider.FailureEvent)
	if !ok || failure.Failure.Diagnostics == nil {
		t.Fatalf("failure %#v", events)
	}
	record := provider.ReadHTTPFailureRecord(*failure.Failure.Diagnostics)
	if record == nil || record.Body != body {
		t.Fatalf("truncated refusal: %#v", record)
	}
	if *failure.Failure.Diagnostics.ProviderRequestID != "header-id" {
		t.Fatal("request header must win")
	}
}

func TestCatalogAndQuota(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	vendor.Route("/codex/models", providertest.MockResponse{Chunks: []string{`{"models":[{"slug":"late","display_name":"Late","visibility":"list","priority":2,"supported_reasoning_levels":[],"experimental_supported_tools":[]},{"slug":"first","display_name":"First","visibility":"list","priority":1,"supported_reasoning_levels":[{"effort":"high"}],"apply_patch_tool_type":null,"input_modalities":["text","image"],"service_tiers":[{"id":"priority","name":"Fast","description":""}]},{"slug":"hidden","display_name":"Hidden","visibility":"hide","priority":0,"supported_reasoning_levels":[]}]}`}}, providertest.MockResponse{Chunks: []string{`{"models":[{"slug":"bad"}]}`}})
	vendor.Route("/wham/usage", providertest.MockResponse{Chunks: []string{`{"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":120,"limit_window_seconds":18000,"reset_at":1900000000},"secondary_window":{"used_percent":12,"limit_window_seconds":604800,"reset_at":1900000000}}}`}})
	p, _ := fixture(t, vendor.Server.URL, codex.TransportSSE)
	catalog, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(catalog.Models) != 2 || *catalog.DefaultModelID != "first" || !*catalog.Models[0].SupportsTools || *catalog.Models[1].SupportsTools || !*catalog.Models[0].SupportsAttachments || !catalog.Models[0].ServiceTiers[0].Fast {
		t.Fatalf("catalog %#v", catalog)
	}
	if vendor.Requests()[0].URI != "/codex/models?client_version=0.153.4" {
		t.Fatalf("URL %s", vendor.Requests()[0].URI)
	}
	if _, err := p.ListModels(t.Context()); err == nil {
		t.Fatal("malformed model accepted")
	}
	snapshot, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.Plan.ID != "pro" || snapshot.Plan.Label != "Pro" || len(snapshot.Windows) != 2 || *snapshot.Windows[0].UsedPercent != 100 || snapshot.Windows[1].Label != "Weekly" {
		t.Fatalf("quota %#v", snapshot)
	}
}

// Cost: scripted metadata requests; a catalog retry must use the refreshed token.
func TestMetadataRefreshAndRefusals(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	vendor.Route("/codex/models", providertest.MockResponse{Status: 401}, providertest.MockResponse{Chunks: []string{`{"models":[]}`}}, providertest.MockResponse{Status: 503})
	vendor.Route("/oauth/token", providertest.MockResponse{Chunks: []string{`{"access_token":"fresh"}`}})
	vendor.Route("/wham/usage", providertest.MockResponse{Status: 503})
	p, _ := fixture(t, vendor.Server.URL, codex.TransportSSE)
	if _, err := p.ListModels(t.Context()); err != nil {
		t.Fatal(err)
	}
	requests := vendor.Requests()
	if len(requests) != 3 || requests[2].Headers.Get("Authorization") != "Bearer fresh" {
		t.Fatalf("catalog refresh %#v", requests)
	}
	if _, err := p.ListModels(t.Context()); err == nil {
		t.Fatal("unavailable catalog accepted")
	}
	if _, err := p.Quota().Probe(t.Context()); err == nil {
		t.Fatal("refused quota accepted")
	}
}

func TestDeviceLoginAndCancellation(t *testing.T) {
	token := providertest.JWT(jsontext.Value(`{"email":"device@example.test","https://api.openai.com/auth":{"chatgpt_account_id":"account-device"}}`))
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Chunks: []string{`{"device_auth_id":"device","usercode":"AB-CD","interval":"0"}`}},
		providertest.MockResponse{Status: 403},
		providertest.MockResponse{Chunks: []string{`{"authorization_code":"code","code_verifier":"verifier"}`}},
		providertest.MockResponse{Chunks: []string{string(provider.JSONBody(map[string]string{"access_token": token, "refresh_token": "refresh", "id_token": token}))}},
	)
	p, pool := fixture(t, vendor.Server.URL, codex.TransportSSE)
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	shown := 0
	info, err := p.Accounts().Login(ctx, func(pending core.LoginPending) {
		shown++
		if pending.VerificationURL != vendor.Server.URL+"/codex/device" || *pending.UserCode != "AB-CD" || pending.ExpiresAt.Millisecond() != 600000 {
			t.Fatalf("pending %#v", pending)
		}
	})
	if err != nil || shown != 1 || info.Label != "device@example.test" {
		t.Fatalf("login %#v %d %v", info, shown, err)
	}
	if len(pool.Entries()) != 2 {
		t.Fatal("login not stored")
	}
	requests := vendor.Requests()
	form, err := url.ParseQuery(string(requests[3].Body))
	if err != nil || form.Get("code_verifier") != "verifier" || form.Get("redirect_uri") != vendor.Server.URL+"/deviceauth/callback" {
		t.Fatalf("exchange %v %v", form, err)
	}
	pending := providertest.NewMockVendor(t, providertest.MockResponse{Chunks: []string{`{"device_auth_id":"device","user_code":"AB-CD"}`}}, providertest.MockResponse{Ending: providertest.Silent})
	p, pool = fixture(t, pending.Server.URL, codex.TransportSSE)
	cancelCtx, stop := context.WithCancel(ctx)
	done := make(chan error, 1)
	go func() { _, err := p.Accounts().Login(cancelCtx, func(core.LoginPending) {}); done <- err }()
	pending.WaitRequests(t, 2)
	stop()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("cancelled login succeeded")
		}
	case <-ctx.Done():
		t.Fatal("login did not cancel")
	}
	if len(pool.Entries()) != 1 {
		t.Fatal("cancelled login published account")
	}
	pending.WaitDisconnect(t)
}

// Cost: two refused loopback logins; neither reaches a token endpoint.
func TestDeviceLoginRejectsMissingCodeAndUnavailableService(t *testing.T) {
	for _, response := range []providertest.MockResponse{
		{Chunks: []string{`{"device_auth_id":"device"}`}},
		{Status: 404},
	} {
		vendor := providertest.NewMockVendor(t, response)
		p, pool := fixture(t, vendor.Server.URL, codex.TransportSSE)
		shown := false
		_, err := p.Accounts().Login(t.Context(), func(core.LoginPending) { shown = true })
		if err == nil || shown || len(pool.Entries()) != 1 || len(vendor.Requests()) != 1 {
			t.Fatalf("invalid login: %v, pending=%v", err, shown)
		}
	}
}

func TestUsageLimitFailureReader(t *testing.T) {
	received, err := core.TimestampFromMillisecond(10000)
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		body string
		want int64
	}{
		{`{"error":{"resets_at":20.125,"resets_in_seconds":99}}`, 20125},
		{`{"event":{"error":{"resets_in_seconds":2.5}}}`, 12500},
		{`{"response":{"error":{"resets_at":30}}}`, 30000},
		{`{"error":{},"event":{"error":{"resets_at":30}}}`, 11000},
		{`{"error":{"resets_at":"30"}}`, 11000},
	} {
		failure := provider.Refused("Codex", 429, http.Header{"Retry-After": {"1"}}, tc.body, codex.ReadFailure, received)
		facts := codex.ReadFailure(*failure.Diagnostics, received)
		if facts.RetryAt == nil || facts.RetryAt.Millisecond() != tc.want {
			t.Fatalf("%s: %#v", tc.body, facts)
		}
	}
}

func TestTransportTimeoutsOnFakeClock(t *testing.T) {
	for _, mode := range []codex.TransportMode{codex.TransportSSE, codex.TransportWebSocket, codex.TransportAuto} {
		t.Run(string(mode), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				vendor := providertest.NewPipeMockVendor(t, providertest.MockResponse{Ending: providertest.Silent}, providertest.MockResponse{}, providertest.MockResponse{})
				_, pool := fixture(t, vendor.Server.URL, mode)
				config := codex.NewConfig(new("one"))
				base, err := url.Parse(vendor.Server.URL)
				if err != nil {
					t.Fatal(err)
				}
				config.BackendURL = *base
				config.AuthURL = *base
				config.Transport = mode
				config.HeaderTimeout = 50 * time.Millisecond
				config.ConnectTimeout = 50 * time.Millisecond
				p, err := codex.New(config, pool, &provider.MemorySnapshots{}, vendor.Server.Client(), providertest.FixedClock{Time: core.UnixEpoch})
				if err != nil {
					t.Fatal(err)
				}
				runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: vendor.Server.Client()})
				if err != nil {
					t.Fatal(err)
				}
				started := time.Now()
				events := providertest.Run(t, runtime, providertest.InferenceRequest())
				if time.Since(started) != 50*time.Millisecond {
					t.Fatalf("deadline took %s", time.Since(started))
				}
				if mode == codex.TransportAuto {
					if len(events) != 1 || events[0] != (provider.Response{}) {
						t.Fatalf("fallback %#v", events)
					}
					events = providertest.Run(t, runtime.Fresh(), providertest.InferenceRequest())
					if len(events) != 1 || events[0] != (provider.Response{}) {
						t.Fatalf("later SSE %#v", events)
					}
					requests := vendor.Requests()
					if len(requests) != 3 || requests[0].Headers.Get("Upgrade") != "websocket" || requests[1].Headers.Get("Upgrade") != "" || requests[2].Headers.Get("Upgrade") != "" {
						t.Fatalf("transport choice %#v", requests)
					}
				} else {
					failure, ok := events[0].(provider.FailureEvent)
					if !ok || failure.Failure.Code != provider.Overloaded || !strings.Contains(failure.Failure.Message, "timed out after 50ms") {
						t.Fatalf("timeout %#v", events)
					}
				}
				vendor.WaitDisconnect(t)
			})
		})
	}
}
func TestDeviceLoginExpiresOnFakeClock(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		vendor := providertest.NewPipeMockVendor(t, providertest.MockResponse{Chunks: []string{`{"device_auth_id":"device","user_code":"AB-CD","interval":600}`}}, providertest.MockResponse{Status: 403}, providertest.MockResponse{Status: 403})
		_, pool := fixture(t, vendor.Server.URL, codex.TransportSSE)
		config := codex.NewConfig(nil)
		base, err := url.Parse(vendor.Server.URL)
		if err != nil {
			t.Fatal(err)
		}
		config.AuthURL = *base
		p, err := codex.New(config, pool, &provider.MemorySnapshots{}, vendor.Server.Client(), providertest.FixedClock{Time: core.UnixEpoch})
		if err != nil {
			t.Fatal(err)
		}
		started := time.Now()
		_, err = p.Accounts().Login(t.Context(), func(core.LoginPending) {})
		if err == nil || !strings.Contains(err.Error(), "timed out") || time.Since(started) != 10*time.Minute || len(pool.Entries()) != 1 {
			t.Fatalf("expiry %v %s", err, time.Since(started))
		}
	})
}

func TestWebSocketIdleAndCancellationCloseReasons(t *testing.T) {
	for _, cancelRun := range []bool{false, true} {
		synctest.Test(t, func(t *testing.T) {
			vendor := providertest.NewPipeMockVendor(t, providertest.MockResponse{Socket: true, Chunks: []string{`{"type":"response.output_text.delta","delta":"first"}`}, Ending: providertest.Open})
			_, pool := fixture(t, vendor.Server.URL, codex.TransportWebSocket)
			config := codex.NewConfig(new("one"))
			base, err := url.Parse(vendor.Server.URL)
			if err != nil {
				t.Fatal(err)
			}
			config.BackendURL = *base
			config.AuthURL = *base
			config.Transport = codex.TransportWebSocket
			config.StreamIdleTimeout = new(50 * time.Millisecond)
			p, err := codex.New(config, pool, &provider.MemorySnapshots{}, vendor.Server.Client(), providertest.FixedClock{Time: core.UnixEpoch})
			if err != nil {
				t.Fatal(err)
			}
			runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: vendor.Server.Client()})
			if err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
			defer cancel()
			events := []provider.ProviderEvent{}
			for event := range runtime.Run(ctx, providertest.InferenceRequest()) {
				events = append(events, event)
				if cancelRun {
					cancel()
				}
			}
			if len(events) == 0 || events[0] != (provider.TextDelta{Text: "first"}) {
				t.Fatalf("events %#v", events)
			}
			reason := "aborted"
			if !cancelRun {
				reason = "idle_timeout"
				if len(events) != 2 {
					t.Fatalf("idle events %#v", events)
				}
				failure, ok := events[1].(provider.FailureEvent)
				if !ok || failure.Failure.Code != provider.Overloaded || !strings.Contains(failure.Failure.Message, "idled for 50ms") {
					t.Fatalf("idle failure %#v", events)
				}
			} else if len(events) != 1 {
				t.Fatalf("events after cancellation %#v", events)
			}
			vendor.WaitDisconnect(t)
			if got := vendor.Requests()[0].SocketClose; !strings.Contains(got, reason) {
				t.Fatalf("close reason %s, want %s", got, reason)
			}
		})
	}
}

func TestWebSocketHandshakeRefreshAndQuota(t *testing.T) {
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Status: 401, Headers: http.Header{"X-Codex-Primary-Used-Percent": {"75"}, "X-Codex-Primary-Window-Minutes": {"300"}}},
		providertest.MockResponse{Chunks: []string{`{"access_token":"renewed"}`}},
		providertest.MockResponse{Socket: true, Chunks: []string{`{"type":"response.completed","response":{}}`}, Ending: providertest.Open},
	)
	p, _ := fixture(t, vendor.Server.URL, codex.TransportWebSocket)
	events := run(t, p)
	if len(events) != 1 || events[0] != (provider.Response{}) {
		t.Fatalf("events %#v", events)
	}
	requests := vendor.Requests()
	if len(requests) != 3 || requests[2].Headers.Get("Authorization") != "Bearer renewed" || requests[2].Headers.Get("Openai-Beta") != "responses_websockets=2026-02-06" {
		t.Fatalf("handshake %#v", requests)
	}
	quota := p.Quota().Latest()
	if quota == nil || len(quota.Windows) != 1 || *quota.Windows[0].UsedPercent != 75 || quota.Windows[0].Label != "5-hour" {
		t.Fatalf("handshake quota %#v", quota)
	}
	vendor.WaitDisconnect(t)
}

// Cost: two scripted runs; cancellation happens before starting or after the first delivered event.
func TestSSECancellationBeforeAndDuringRun(t *testing.T) {
	for _, before := range []bool{true, false} {
		vendor := providertest.NewMockVendor(t, providertest.MockResponse{Chunks: []string{providertest.SSEBody(jsontext.Value(`{"type":"response.output_text.delta","delta":"first"}`))}, Ending: providertest.Open})
		p, _ := fixture(t, vendor.Server.URL, codex.TransportSSE)
		runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: vendor.Server.Client()})
		if err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
		if before {
			cancel()
		}
		count := 0
		for event := range runtime.Run(ctx, providertest.InferenceRequest()) {
			if event != (provider.TextDelta{Text: "first"}) {
				t.Fatalf("event %#v", event)
			}
			count++
			cancel()
		}
		cancel()
		if before {
			if count != 0 || len(vendor.Requests()) != 0 {
				t.Fatal("pre-cancelled request was sent")
			}
		} else {
			if count != 1 {
				t.Fatalf("event count %d", count)
			}
			vendor.WaitDisconnect(t)
		}
	}
}

// Cost: five scripted catalog responses; null is present for capability detection.
func TestCatalogCapabilityPresence(t *testing.T) {
	for _, tc := range []struct {
		field   string
		tools   *bool
		invalid bool
	}{
		{field: ""},
		{field: `,"apply_patch_tool_type":null`, tools: new(true)},
		{field: `,"web_search_tool_type":"web"`, tools: new(true)},
		{field: `,"apply_patch_tool_type":null,"experimental_supported_tools":[]`, tools: new(false)},
		{field: `,"apply_patch_tool_type":false`, invalid: true},
	} {
		vendor := providertest.NewMockVendor(t, providertest.MockResponse{Chunks: []string{`{"models":[{"slug":"one","display_name":"One","visibility":"list","priority":0,"supported_reasoning_levels":[]` + tc.field + `}]}`}})
		p, _ := fixture(t, vendor.Server.URL, codex.TransportSSE)
		catalog, err := p.ListModels(t.Context())
		if tc.invalid {
			if err == nil {
				t.Fatal("wrong capability kind accepted")
			}
			continue
		}
		if err != nil {
			t.Fatal(err)
		}
		got := catalog.Models[0].SupportsTools
		if (got == nil) != (tc.tools == nil) || got != nil && *got != *tc.tools {
			t.Fatalf("capability for %s: %v", tc.field, got)
		}
	}
}
