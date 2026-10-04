package claudecode_test

import (
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/claudecode"
)

func TestSetupTokenAccountAndPlacementRequirement(t *testing.T) {
	p, pool := testProvider(t, "http://127.0.0.1:9/catalog", "http://127.0.0.1:9/usage")
	signedIn, _, err := pool.Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	staged := claudecode.New(
		claudecode.NewConfig("entry-1", "Claude", nil),
		pool,
		&provider.MemorySnapshots{},
		provider.NewModelsDevClient(http.DefaultClient, "http://127.0.0.1:9/catalog", testClock()),
		http.DefaultClient,
		testClock(),
	)
	message := "No Claude Code account is signed in"
	equal(t, staged.AuthStatus(t.Context()), &core.Unauthenticated{Message: &message})
	accounts := staged.Accounts()
	equal(t, accounts.Capability(), provider.AccountsCapability{Add: true})
	_, err = accounts.Login(t.Context(), func(core.LoginPending) {})
	if !errors.Is(err, provider.ErrLoginUnsupported) {
		t.Fatal(err)
	}
	token, err := provider.NewSecret(testToken)
	if err != nil {
		t.Fatal(err)
	}
	added, err := accounts.Add(t.Context(), provider.AddAccount{SetupToken: token})
	if err != nil {
		t.Fatal(err)
	}
	// The same setup token is the same account, not a second one.
	equal(t, added.ID, signedIn)
	if !strings.HasPrefix(added.Label, "claude-") || len(added.Label) != 15 {
		t.Fatal(added.Label)
	}
	equal(t, added.Detail, (*string)(nil))
	list, err := pool.List(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, len(list), 1)
	active, _, err := pool.Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, active, added.ID)
	document, _, err := pool.Document(added.ID).Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	checkJSON(t, []byte(document.Text), `{"accessToken":"`+testToken+`"}`)
	equal(t, p.AuthStatus(t.Context()), &core.Authenticated{AccountLabel: &added.Label})
	equal(t, p.Capabilities().ProcessHost, true)
	if _, ok := p.RuntimeState().(*core.RuntimeUnknown); !ok {
		t.Fatal(p.RuntimeState())
	}
	_, err = p.Runtime(provider.RuntimeEnv{})
	if err == nil || err.Error() != "Claude needs a Host that runs processes" {
		t.Fatal(err)
	}
	replaced, err := pool.Document(added.ID).
		Replace(t.Context(), `{"accessToken":"x","refreshToken":"y"}`, document.Version)
	if err != nil || !replaced {
		t.Fatal(replaced, err)
	}
	if _, ok := p.AuthStatus(t.Context()).(*core.AuthError); !ok {
		t.Fatal("corrupt document accepted")
	}
}

func vendorJSON(v *providertest.MockVendor, body string) {
	v.Respond(
		providertest.MockResponse{
			Status:  200,
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  [][]byte{[]byte(body)},
		},
	)
}

func TestQuotaProbeAndCLIObservation(t *testing.T) {
	vendor := providertest.StartVendor(t)
	p, _ := testProvider(t, "http://127.0.0.1:9/catalog", vendor.URL("/api/oauth/usage"))
	vendorJSON(
		vendor,
		`{"five_hour":{"utilization":12,`+
			`"resets_at":"2026-09-24T10:00:00.000Z"},`+
			`"seven_day":{"utilization":"lots"},"seven_day_opus":null,`+
			`"limits":[{"kind":"session","percent":12},{"percent":50},`+
			`{"kind":"weekly_scoped","percent":100,"severity":"critical",`+
			`"resets_at":"2026-09-28T08:00:00Z",`+
			`"scope":{"model":{"display_name":"Fable"}}},`+
			`{"kind":"monthly_credits","percent":85,"severity":"sideways"}]}`,
	)
	snapshot, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	req := vendor.Requests()[0]
	equal(t, req.Header("Authorization"), "Bearer "+testToken)
	equal(t, req.Header("anthropic-beta"), "oauth-2025-04-20")
	equal(t, req.URI, "/api/oauth/usage")
	equal(t, snapshot.Source, core.SnapshotSource("probe"))
	equal(t, snapshot.Plan, (*core.QuotaPlan)(nil))
	var ids []string
	for _, w := range snapshot.Windows {
		ids = append(ids, w.ID)
	}
	equal(t, ids, []string{"five_hour", "limit:weekly_scoped:Fable", "limit:monthly_credits"})
	equal(t, snapshot.Windows[0].Label, "5h session")
	equal(t, *snapshot.Windows[0].UsedPercent, 12.0)
	equal(t, *snapshot.Windows[0].ResetsAt, core.Timestamp("2026-09-24T10:00:00.000Z"))
	equal(t, *snapshot.Windows[1].Severity, core.QuotaSeverity("critical"))
	equal(t, snapshot.Windows[1].Label, "weekly_scoped (Fable)")
	equal(t, snapshot.Windows[1].Scope.Kind, "model")
	equal(t, *snapshot.Windows[1].Scope.Label, "Fable")
	equal(t, *snapshot.Windows[2].Severity, core.QuotaSeverity("warning"))
	vendorJSON(vendor, `["five_hour"]`)
	_, err = p.Quota().Probe(t.Context())
	if err == nil {
		t.Fatal("probe accepted an unreadable usage answer")
	}
	vendor.Respond(providertest.MockResponse{Status: 401, Chunks: [][]byte{[]byte("token expired")}})
	_, err = p.Quota().Probe(t.Context())
	if err == nil || err.Error() != "Claude usage request failed (401): token expired" {
		t.Fatalf("quota refusal: %v", err)
	}
	placement := &scriptedPlacement{t: t, setup: func(c *scriptedCLI) {
		c.onWrite = func(_ map[string]json.RawMessage) {
			c.say(
				`{"type":"rate_limit_event",` +
					`"rate_limit_info":{"status":"allowed_warning",` +
					`"resetsAt":1790855225,"rateLimitType":"seven_day_opus",` +
					`"utilization":0.97,` +
					`"unifiedWindows":{"five_hour":{"utilization":0.33,` +
					`"resetsAt":1790855225},"seven_day":{"utilization":0.5,` +
					`"resetsAt":"soon"},` +
					`"seven_day_overage_included":{"utilization":0.1,"resetsAt":1790855225}}}}`,
			)
			c.result(1, 1)
		}
	}}
	runtime := p.ProcessRuntime(placement)
	defer func() {
		if err := runtime.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	collect(t.Context(), runtime, request(user("hi")))
	snapshot = p.Quota().Latest()
	equal(t, snapshot.Source, core.SnapshotSource("observation"))
	equal(t, len(snapshot.Windows), 5)
	ids = nil
	for _, w := range snapshot.Windows {
		ids = append(ids, w.ID)
		switch w.ID {
		case "five_hour":
			equal(t, *w.UsedPercent, 33.0)
			equal(t, *w.ResetsAt, core.Timestamp("2026-10-01T11:47:05.000Z"))
		case "seven_day":
			equal(t, *w.UsedPercent, 50.0)
			equal(t, w.ResetsAt, (*core.Timestamp)(nil))
		case "seven_day_opus":
			equal(t, *w.UsedPercent, 97.0)
			equal(t, *w.Severity, core.QuotaSeverity("critical"))
		}
	}
	equal(
		t,
		ids,
		[]string{"five_hour", "limit:weekly_scoped:Fable", "limit:monthly_credits", "seven_day", "seven_day_opus"},
	)
}

func TestClaudeCatalogMinimumOrderingAndThinking(t *testing.T) {
	vendor := providertest.StartVendor(t)
	p, _ := testProvider(t, vendor.URL("/api.json"), "http://127.0.0.1:9/usage")
	models := map[string]any{}
	for _, id := range []string{
		"claude-haiku-4-5",
		"claude-sonnet-4-6",
		"claude-3-5-sonnet-20241022",
		"claude-opus-4-8",
		"claude-opus-4-6",
		"claude-sonnet-4-20250514",
		"claude-mystery",
		"gpt-4o",
	} {
		name := id
		if id == "claude-opus-4-8" {
			name = "Claude Opus 4.8"
		}
		models[id] = map[string]any{
			"name":              name,
			"attachment":        true,
			"reasoning":         true,
			"tool_call":         true,
			"reasoning_options": []any{map[string]any{"type": "effort", "values": []string{"low", "medium", "high"}}},
			"limit":             map[string]int{"context": 1000000, "output": 128000},
			"cost":              map[string]float64{"input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25},
		}
	}
	models["claude-newfamily-5"] = map[string]any{"name": "Claude Newfamily 5"}
	data, err := provider.JSONBody(
		map[string]any{
			"anthropic": map[string]any{
				"id":     "anthropic",
				"name":   "Anthropic",
				"npm":    "@ai-sdk/anthropic",
				"models": models,
			},
			"openai": map[string]any{"id": "openai", "name": "OpenAI", "models": map[string]any{}},
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	vendorJSON(vendor, string(data))
	catalog, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	var ids []string
	for _, model := range catalog.Models {
		ids = append(ids, model.ID)
		equal(t, *model.CanDisableThinking, false)
	}
	equal(t, ids, []string{"claude-opus-4-8", "claude-opus-4-6", "claude-sonnet-4-6", "claude-newfamily-5"})
	equal(t, catalog.Warnings, []string{"Skipped Claude model with unparseable version: claude-mystery"})
	equal(t, catalog.Models[0].DisplayName, "Claude Opus 4.8")
	equal(t, *catalog.Models[0].ContextWindow, uint32(1000000))
	equal(t, catalog.Models[3].SupportsTools, (*bool)(nil))
	equal(t, catalog.Models[3].ContextWindow, (*uint32)(nil))
	vendorJSON(vendor, `{"openai":{"id":"openai","name":"OpenAI","models":{}}}`)
	_, err = p.ListModels(t.Context())
	if err == nil || err.Error() != "models.dev does not list the anthropic vendor" {
		t.Fatal(err)
	}
}
