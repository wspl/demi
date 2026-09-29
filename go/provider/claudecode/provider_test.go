package claudecode_test

import (
	"encoding/json/jsontext"
	"errors"
	"net/url"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/claudecode"
	"github.com/wspl/demi/go/provider/providertest"
)

// Cost: local HTTP only. No CLI process or model account is used.
func TestAccountCatalogAndQuota(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	vendor.Route("/models", providertest.MockResponse{Chunks: []string{`{"anthropic":{"id":"anthropic","name":"Anthropic","models":{"claude-sonnet-4-6":{"name":"Sonnet"},"claude-opus-4-8":{"name":"Opus"},"claude-opus-4-6":{"name":"Older opus"},"claude-sonnet-4-20250514":{"name":"Old"},"claude-next":{"name":"Unknown"}}}}`}})
	vendor.Route("/usage", providertest.MockResponse{Chunks: []string{`{"five_hour":{"utilization":25,"resets_at":"2030-01-01T00:00:00Z"},"seven_day":{"utilization":"bad"},"limits":[{"kind":"session","percent":50},{"kind":"burst","percent":95,"severity":"warning","scope":{"model":{"display_name":"Opus"}}}]}`}}, providertest.MockResponse{Chunks: []string{`[]`}})
	pool := provider.NewMemoryCredentialPool()
	clock := providertest.FixedClock{Time: core.UnixEpoch}
	models := provider.NewModelsDevClient(vendor.Server.Client(), vendor.Server.URL+"/models", clock)
	config := claudecode.NewConfig("entry", "Claude", nil)
	usage, err := url.Parse(vendor.Server.URL + "/usage")
	if err != nil {
		t.Fatal(err)
	}
	config.UsageURL = *usage
	p := claudecode.New(config, pool, &provider.MemorySnapshots{}, models, vendor.Server.Client(), clock)
	if !p.Capabilities().ProcessHost {
		t.Fatal("process host not required")
	}
	if _, err := p.Runtime(provider.RuntimeEnv{}); err == nil {
		t.Fatal("runtime without placement accepted")
	} else {
		var required *provider.ProcessHostRequired
		if !errors.As(err, &required) {
			t.Fatal(err)
		}
	}
	if _, ok := p.AuthStatus(t.Context()).(core.AuthStateUnauthenticated); !ok {
		t.Fatal("missing account authenticated")
	}
	token, err := provider.NewSecret("setup-token")
	if err != nil {
		t.Fatal(err)
	}
	first, err := p.Accounts().Add(t.Context(), provider.AddAccount{SetupToken: token})
	if err != nil {
		t.Fatal(err)
	}
	again, err := p.Accounts().Add(t.Context(), provider.AddAccount{SetupToken: token})
	if err != nil || again.ID != first.ID || len(pool.Entries()) != 1 {
		t.Fatalf("dedup %v %v", again, err)
	}
	if !strings.HasPrefix(first.Label, "claude-") || strings.Contains(first.Label, "setup") {
		t.Fatalf("label %s", first.Label)
	}
	config.Account = &first.ID
	p = claudecode.New(config, pool, &provider.MemorySnapshots{}, models, vendor.Server.Client(), clock)
	if _, ok := p.AuthStatus(t.Context()).(core.AuthStateAuthenticated); !ok {
		t.Fatal("stored token not authenticated")
	}
	catalog, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(catalog.Models) != 3 || catalog.Models[0].ID != "claude-opus-4-8" || catalog.Models[1].ID != "claude-opus-4-6" || catalog.Models[2].ID != "claude-sonnet-4-6" || len(catalog.Warnings) != 1 || *catalog.Models[0].CanDisableThinking {
		t.Fatalf("catalog %#v", catalog)
	}
	snapshot, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(snapshot.Windows) != 2 || snapshot.Windows[0].ID != "five_hour" || *snapshot.Windows[0].UsedPercent != 25 || snapshot.Windows[1].ID != "limit:burst:Opus" || snapshot.Windows[1].Scope.Kind != "model" {
		t.Fatalf("quota %#v", snapshot)
	}
	requests := vendor.Requests()
	if requests[1].Headers.Get("Authorization") != "Bearer setup-token" || requests[1].Headers.Get("Anthropic-Beta") != "oauth-2025-04-20" {
		t.Fatal("quota credentials missing")
	}
	p.Quota().Observe(provider.Observation{CLILine: jsontext.Value(`{"message":{"rate_limits":{"five_hour":{"used_percentage":70},"seven_day_opus":{"utilization":10}}}}`)})
	snapshot = p.Quota().Latest()
	if len(snapshot.Windows) != 3 || *snapshot.Windows[0].UsedPercent != 70 || snapshot.Windows[2].ID != "seven_day_opus" {
		t.Fatalf("observation %#v", snapshot)
	}
	if _, err := p.Quota().Probe(t.Context()); err == nil {
		t.Fatal("nonobject usage accepted")
	}
}
