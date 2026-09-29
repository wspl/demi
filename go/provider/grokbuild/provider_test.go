package grokbuild_test

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"reflect"
	"slices"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/grokbuild"
	"github.com/wspl/demi/go/provider/providertest"
)

// fixture gives each proxy scenario an isolated stored account.
func fixture(t *testing.T, vendor *providertest.MockVendor, refresh bool) (*grokbuild.Provider, *provider.MemoryCredentialPool) {
	t.Helper()
	token, err := provider.NewSecret("access")
	if err != nil {
		t.Fatal(err)
	}
	secret := grokbuild.SecretDocument{AccessToken: token, Issuer: vendor.Server.URL, ClientID: "test-client", UserID: new("user"), Email: new("stored@example.test"), Principal: &grokbuild.Principal{Kind: "Team", ID: "team"}}
	if refresh {
		secret.RefreshToken = &token
	}
	raw, err := secret.JSON()
	if err != nil {
		t.Fatal(err)
	}
	pool := provider.NewMemoryCredentialPool()
	if err := pool.Write(t.Context(), provider.AccountMeta{ID: "one", Label: "account"}, string(raw)); err != nil {
		t.Fatal(err)
	}
	return configured(t, vendor, pool, new("one")), pool
}
func configured(t *testing.T, vendor *providertest.MockVendor, pool provider.CredentialPool, account *string) *grokbuild.Provider {
	t.Helper()
	config := grokbuild.NewConfig(account)
	base, err := url.Parse(vendor.Server.URL)
	if err != nil {
		t.Fatal(err)
	}
	config.ProxyURL = *base
	config.IssuerURL = *base
	p, err := grokbuild.New(config, pool, &provider.MemorySnapshots{}, vendor.Server.Client(), providertest.FixedClock{Time: core.UnixEpoch})
	if err != nil {
		t.Fatal(err)
	}
	return p
}
func runtime(t *testing.T, p *grokbuild.Provider) provider.ProviderRuntime {
	t.Helper()
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: http.DefaultClient})
	if err != nil {
		t.Fatal(err)
	}
	return runtime
}

// Cost: loopback only; contexts bound missing events at ten seconds.
func TestRequestRefreshStreamAndQuota(t *testing.T) {
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Status: 401, Headers: http.Header{"X-Ratelimit-Limit-Requests": {"10"}, "X-Ratelimit-Remaining-Requests": {"0"}}},
		providertest.MockResponse{Chunks: []string{`{"access_token":"new-token","expires_in":"3600"}`}},
		providertest.MockResponse{Chunks: []string{providertest.SSEBody(jsontext.Value(`{"choices":[{"delta":{"reasoning_content":"think","content":"answer","tool_calls":[{"index":0,"id":"call","function":{"name":"read","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"prompt_tokens_details":{"cached_tokens":3}}}`))}},
	)
	p, pool := fixture(t, vendor, true)
	request := providertest.InferenceRequest()
	request.OutputLimit = new(uint32(4))
	request.ServiceTierID = new("priority")
	request.Tools = []provider.ToolDefinition{{Name: "read", InputSchema: jsontext.Value(`{"type":"object"}`)}}
	events := providertest.Run(t, runtime(t, p), request)
	want := []provider.ProviderEvent{provider.ThinkingStart{}, provider.ThinkingDelta{Text: "think"}, provider.TextDelta{Text: "answer"}, provider.ToolCall{ToolUseID: "call", ToolName: "read", Input: jsontext.Value(`{}`)}, provider.Response{Usage: core.TokenUsage{InputTokens: 9, OutputTokens: 4, CacheReadTokens: 3}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events %#v", events)
	}
	requests := vendor.Requests()
	if len(requests) != 3 || requests[0].URI != "/chat/completions" || requests[1].URI != "/oauth2/token" {
		t.Fatalf("requests %#v", requests)
	}
	form, err := url.ParseQuery(string(requests[1].Body))
	if err != nil || form.Get("principal_type") != "Team" || form.Get("principal_id") != "team" || form.Get("refresh_token") != "access" {
		t.Fatalf("refresh %v %v", form, err)
	}
	if !slices.Equal(requests[0].Body, requests[2].Body) {
		t.Fatal("retry changed body")
	}
	for header, want := range map[string]string{"Authorization": "Bearer new-token", "X-Xai-Token-Auth": "xai-grok-cli", "X-Grok-Client-Version": "1.0.5", "X-Userid": "user", "X-Email": "stored@example.test", "X-Grok-Session-Id": "session-1", "X-Grok-Req-Id": "request-1", "X-Grok-Turn-Idx": "turn-1"} {
		if got := requests[2].Headers.Get(header); got != want {
			t.Fatalf("%s=%s", header, got)
		}
	}
	var body map[string]jsontext.Value
	if err := json.Unmarshal(requests[0].Body, &body); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"max_tokens", "max_completion_tokens", "service_tier"} {
		if _, ok := body[key]; ok {
			t.Fatalf("unexpected %s", key)
		}
	}
	if string(body["tool_choice"]) != `"auto"` {
		t.Fatal("tools unavailable")
	}
	if !strings.Contains(pool.Entries()[0].Secret, "new-token") {
		t.Fatal("refresh not stored")
	}
	snapshot := p.Quota().Latest()
	if snapshot == nil || len(snapshot.Windows) != 1 || *snapshot.Windows[0].UsedPercent != 100 {
		t.Fatalf("refusal quota %#v", snapshot)
	}
}
func TestCatalogAndCreditProbe(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	vendor.Route("/models", providertest.MockResponse{Chunks: []string{`{"data":[{"id":"model","name":"Model","context_window":100,"reasoning_efforts":[{"id":"low"},{"value":"high","default":true}]},{}]}`}}, providertest.MockResponse{Chunks: []string{`[{"model":"bare"}]`}}, providertest.MockResponse{Chunks: []string{`{"data":[{"id":""}]}`}}, providertest.MockResponse{Chunks: []string{`[{"model":"bare"},{"id":""}]`}}, providertest.MockResponse{Chunks: []string{`[{"model":"bare","context_window":0}]`}})
	vendor.Route("/user", providertest.MockResponse{Chunks: []string{`{"subscriptionTier":"pro","email":"server@example.test"}`}})
	vendor.Route("/billing", providertest.MockResponse{Chunks: []string{`{"config":{"monthlyLimit":{"val":20},"used":15,"onDemandCap":10,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2030-01-01T00:00:00Z"}}}`}})
	p, _ := fixture(t, vendor, false)
	catalog, err := p.ListModels(t.Context())
	if err != nil || len(catalog.Models) != 1 || *catalog.Models[0].DefaultThinkingEffort != "high" || *catalog.DefaultModelID != "model" {
		t.Fatalf("catalog %#v %v", catalog, err)
	}
	catalog, err = p.ListModels(t.Context())
	if err != nil || *catalog.DefaultModelID != "bare" {
		t.Fatalf("bare %#v %v", catalog, err)
	}
	// A model breaks its rules alike in the envelope and in the bare list.
	for range 3 {
		_, err := p.ListModels(t.Context())
		var refused *provider.CatalogError
		if !errors.As(err, &refused) || refused.Kind != provider.CatalogInvalid {
			t.Fatalf("malformed model accepted: %v", err)
		}
	}
	snapshot, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.Plan.ID != "pro" || *snapshot.AccountLabel != "stored@example.test" || len(snapshot.Windows) != 2 || snapshot.Windows[0].ID != "weekly" || *snapshot.Windows[0].UsedPercent != 75 || *snapshot.Windows[1].Limit != 10 {
		t.Fatalf("probe %#v", snapshot)
	}
	paths := []string{}
	for _, request := range vendor.Requests() {
		paths = append(paths, request.URI)
	}
	if !slices.Contains(paths, "/billing?format=credits") || !slices.Contains(paths, "/user?include=subscription") {
		t.Fatalf("paths %v", paths)
	}
}

// Cost: one parallel pair of metadata reads; optional malformed billing fields are absent.
// Cost: two pairs of held loopback responses; waits are released by events, not elapsed time.
func TestQuotaProbeStartsBothRequestsBeforeEitherFinishes(t *testing.T) {
	for _, cancelled := range []bool{false, true} {
		t.Run(fmt.Sprintf("cancelled=%v", cancelled), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				release := make(chan struct{})
				vendor := providertest.NewPipeMockVendor(t)
				vendor.Route("/user", providertest.MockResponse{Chunks: []string{`{"subscriptionTier":"pro"}`}, Release: release})
				vendor.Route("/billing", providertest.MockResponse{Chunks: []string{`{"config":{}}`}, Release: release})
				p, _ := fixture(t, vendor, false)
				guard, stop := context.WithTimeout(t.Context(), 2*time.Minute)
				defer stop()
				ctx, cancel := context.WithCancel(guard)
				defer cancel()
				done := make(chan error, 1)
				go func() { _, err := p.Quota().Probe(ctx); done <- err }()
				vendor.WaitRequests(t, 2)
				if cancelled {
					cancel()
				} else {
					close(release)
				}
				select {
				case err := <-done:
					if (err != nil) != cancelled {
						t.Fatalf("probe: %v", err)
					}
				case <-guard.Done():
					t.Fatal("quota probe did not finish and join its workers")
				}
			})
		})
	}
}

func TestUnmeteredQuotaIgnoresOddFields(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	vendor.Route("/user", providertest.MockResponse{Chunks: []string{`{"subscriptionTier":"free","email":[]}`}})
	vendor.Route("/billing", providertest.MockResponse{Chunks: []string{`{"config":{"monthlyLimit":"unknown","used":{},"onDemandCap":false}}`}})
	p, _ := fixture(t, vendor, false)
	snapshot, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.Plan.ID != "free" || len(snapshot.Windows) != 0 {
		t.Fatalf("unmetered quota %#v", snapshot)
	}
}

func TestCancellationClosesStream(t *testing.T) {
	vendor := providertest.NewMockVendor(t, providertest.MockResponse{Chunks: []string{"data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n"}, Ending: providertest.Open})
	p, _ := fixture(t, vendor, false)
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	count := 0
	for event := range runtime(t, p).Run(ctx, providertest.InferenceRequest()) {
		if event != (provider.TextDelta{Text: "first"}) {
			t.Fatalf("event %#v", event)
		}
		count++
		cancel()
	}
	if count != 1 {
		t.Fatalf("events %d", count)
	}
	vendor.WaitDisconnect(t)
}
func TestDeviceLogin(t *testing.T) {
	// Keep the server outside the fake-time bubble: network idle polling must
	// not hold up its clock. Close idle client sockets before the next wait.
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Headers: http.Header{"Connection": {"close"}}, Chunks: []string{`{"device_code":"device","user_code":"AB-CD","verification_uri":"https://example.test/verify","interval":0}`}},
		providertest.MockResponse{Headers: http.Header{"Connection": {"close"}}, Status: 400, Chunks: []string{`{"error":"slow_down"}`}},
		providertest.MockResponse{Headers: http.Header{"Connection": {"close"}}, Chunks: []string{`{"access_token":"token","refresh_token":"refresh","expires_in":3600}`}},
		providertest.MockResponse{Headers: http.Header{"Connection": {"close"}}, Chunks: []string{`{"userId":"proxy-user","email":"proxy@example.test","principal_type":"Organization","principal_id":"org"}`}},
	)
	synctest.Test(t, func(t *testing.T) {
		pool := provider.NewMemoryCredentialPool()
		p := configured(t, vendor, pool, nil)
		started := time.Now()
		pendingCount := 0
		info, err := p.Accounts().Login(t.Context(), func(pending core.LoginPending) {
			pendingCount++
			if pending.VerificationURL != "https://example.test/verify" || *pending.UserCode != "AB-CD" || pending.ExpiresAt.Millisecond() != 600000 {
				t.Fatalf("pending %#v", pending)
			}
		})
		if err != nil {
			t.Fatal(err)
		}
		if pendingCount != 1 || info.Label != "proxy@example.test" || time.Since(started) != 7*time.Second {
			t.Fatalf("login %#v %d %s", info, pendingCount, time.Since(started))
		}
		entries := pool.Entries()
		if len(entries) != 1 || !strings.Contains(entries[0].Secret, `"principal":{"kind":"Organization","id":"org"}`) {
			t.Fatalf("stored %#v", entries)
		}
		requests := vendor.Requests()
		form, err := url.ParseQuery(string(requests[0].Body))
		if err != nil || form.Get("referrer") != "grok-build" || form.Get("scope") != grokbuild.Scope || requests[0].Headers.Get("X-Grok-Client-Surface") != "ui" {
			t.Fatalf("login request %v %v", form, err)
		}
	})
}

func TestDeviceLoginRefusalExpiryCancellationAndTeam(t *testing.T) {
	for _, scenario := range []string{"expiry", "unsafe-uri", "refused", "cancelled", "team"} {
		t.Run(scenario, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				code := `{"device_code":"device","user_code":"AB-CD","verification_uri":"https://example.test/verify","interval":1}`
				switch scenario {
				case "expiry":
					// The vendor's interval outlasts the login, which still ends
					// ten minutes after its code is shown.
					code = `{"device_code":"device","user_code":"AB-CD","verification_uri":"https://example.test/verify","interval":3600}`
				case "unsafe-uri":
					code = `{"device_code":"device","user_code":"AB-CD","verification_uri":"javascript:alert(1)"}`
				}
				token := providertest.JWT(jsontext.Value(`{"principalType":"Team","principalId":"team-id"}`))
				idToken := providertest.JWT(jsontext.Value(`{"sub":"user","email":"person@example.test"}`))
				confirmed := string(provider.JSONBody(map[string]string{"access_token": token, "id_token": idToken}))
				answer := providertest.MockResponse{Status: 400, Chunks: []string{`{"error":"access_denied"}`}}
				if scenario == "team" {
					answer = providertest.MockResponse{Chunks: []string{confirmed}}
				}
				vendor := providertest.NewPipeMockVendor(t, providertest.MockResponse{Chunks: []string{code}}, answer, providertest.MockResponse{Status: 503})
				pool := provider.NewMemoryCredentialPool()
				p := configured(t, vendor, pool, nil)
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				started := time.Now()
				info, err := p.Accounts().Login(ctx, func(core.LoginPending) {
					if scenario == "cancelled" {
						cancel()
					}
				})
				if scenario == "team" {
					if err != nil || info.Label != "team-id" || len(pool.Entries()) != 1 {
						t.Fatalf("team %#v %v", info, err)
					}
					var secret grokbuild.SecretDocument
					if err := json.Unmarshal([]byte(pool.Entries()[0].Secret), &secret); err != nil {
						t.Fatal(err)
					}
					if secret.Email != nil || *secret.UserID != "team-id" || secret.Principal.Kind != "Team" {
						t.Fatalf("team identity %#v", secret)
					}
				} else {
					if err == nil || len(pool.Entries()) != 0 {
						t.Fatalf("refusal %v accounts %v", err, pool.Entries())
					}
					if scenario == "expiry" && (!strings.Contains(err.Error(), "timed out") || time.Since(started) != 10*time.Minute) {
						t.Fatalf("expiry %v %s", err, time.Since(started))
					}
					if scenario == "cancelled" && len(vendor.Requests()) != 1 {
						t.Fatal("cancelled login polled")
					}
				}
			})
		})
	}
}

// Another refresher holds the account's turn and stores new tokens once the
// request with the old ones has gone out: the refused run takes them instead
// of refreshing again.
// Cost: two loopback requests; a two-minute context guards the run.
func TestRefusedTokenReplacedMeanwhileIsNotRefreshedAgain(t *testing.T) {
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Status: 401},
		providertest.MockResponse{Chunks: []string{providertest.SSEBody(jsontext.Value(`{"choices":[{"delta":{},"finish_reason":"stop"}]}`))}},
	)
	p, pool := fixture(t, vendor, true)
	document := pool.Document("one")
	turn, err := document.RefreshTurn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	running := runtime(t, p)
	ran := make(chan []provider.ProviderEvent, 1)
	go func() { ran <- slices.Collect(running.Run(ctx, providertest.InferenceRequest())) }()
	vendor.WaitRequests(t, 1)
	revision, err := document.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	access, err := provider.NewSecret("rotated-token")
	if err != nil {
		t.Fatal(err)
	}
	refresh, err := provider.NewSecret("refresh-2")
	if err != nil {
		t.Fatal(err)
	}
	rotated, err := grokbuild.SecretDocument{AccessToken: access, RefreshToken: &refresh, Issuer: vendor.Server.URL, ClientID: "test-client", UserID: new("user"), Email: new("stored@example.test"), Principal: &grokbuild.Principal{Kind: "Team", ID: "team"}}.JSON()
	if err != nil {
		t.Fatal(err)
	}
	if kept, err := document.Replace(t.Context(), string(rotated), revision.Version); err != nil || !kept {
		t.Fatalf("replace %v %v", kept, err)
	}
	turn.Release()
	events := <-ran
	if len(events) != 1 || !reflect.DeepEqual(events[0], provider.Response{}) {
		t.Fatalf("events %#v", events)
	}
	requests := vendor.Requests()
	if len(requests) != 2 || requests[1].URI != "/chat/completions" || requests[1].Headers.Get("Authorization") != "Bearer rotated-token" {
		t.Fatalf("requests %#v", requests)
	}
}
