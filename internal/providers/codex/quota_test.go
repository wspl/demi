package codex_test

import (
	"errors"
	"net/http"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestResponseQuotaIncludingRefusals(t *testing.T) {
	v, _, p := setup(t)
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: v.Client()})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := runtime.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	for _, used := range []string{"35.5", "100"} {
		a := completed()
		if used == "100" {
			a = answer(429, "{}")
		}
		a.Headers = http.Header{"X-Codex-Primary-Used-Percent": {used}, "X-Codex-Primary-Window-Minutes": {"300"}, "X-Codex-Primary-Reset-At": {"1700000000"}, "X-Codex-Secondary-Used-Percent": {"10"}, "X-Codex-Secondary-Window-Minutes": {"10080"}}
		v.RespondAt(responses, a)
		providertest.Run(t.Context(), t, runtime, providertest.InferenceRequest())
		snapshot := p.Quota().Latest()
		equal(t, len(snapshot.Windows), 2)
		equal(t, snapshot.Windows[0].Label, "5-hour")
		equal(t, *snapshot.Windows[0].ResetsAt, core.Timestamp("2023-11-14T22:13:20.000Z"))
		equal(t, snapshot.Windows[1].Label, "Weekly")
		equal(t, snapshot.Windows[0].ID, "primary")
		equal(t, snapshot.Windows[1].ID, "secondary")
		equal(t, *snapshot.Windows[1].UsedPercent, float64(10))
		equal(t, snapshot.Windows[1].ResetsAt, (*core.Timestamp)(nil))
		expected := 35.5
		if used == "100" {
			expected = 100
			equal(t, *snapshot.Windows[0].Severity, core.QuotaSeverity("critical"))
		}
		equal(t, *snapshot.Windows[0].UsedPercent, expected)
		equal(t, snapshot.Source, core.SnapshotSource("observation"))
	}
}
func TestFreeUsageProbe(t *testing.T) {
	v, _, p := setup(t)
	v.RespondAt("/backend-api/wham/usage", answer(200, `{"plan_type":"self_serve_business_usage_based","rate_limit":{"allowed":false,"limit_reached":true,"primary_window":{"used_percent":100,"limit_window_seconds":18000,"reset_after_seconds":60,"reset_at":1700000000},"secondary_window":{"used_percent":41,"limit_window_seconds":172800,"reset_after_seconds":900,"reset_at":1700500000}}}`))
	equal(t, *p.Quota().ProbeCost(), provider.ProbeFree)
	snapshot, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, snapshot.Plan.ID, "self_serve_business_usage_based")
	equal(t, snapshot.Plan.Label, "Self serve business usage based")
	equal(t, *snapshot.AccountLabel, "dev@example.com")
	equal(t, len(snapshot.Windows), 2)
	equal(t, snapshot.Windows[0].ID, "primary")
	equal(t, snapshot.Windows[1].ID, "secondary")
	equal(t, *snapshot.Windows[0].UsedPercent, float64(100))
	equal(t, *snapshot.Windows[0].ResetsAt, core.Timestamp("2023-11-14T22:13:20.000Z"))
	equal(t, *snapshot.Windows[1].ResetsAt, core.Timestamp("2023-11-20T17:06:40.000Z"))
	equal(t, snapshot.Windows[0].Label, "5-hour")
	equal(t, snapshot.Windows[1].Label, "2-day")
	equal(t, p.Quota().Latest(), snapshot)
	equal(t, *snapshot.Windows[1].UsedPercent, float64(41))
	equal(t, len(v.Requests()), 1)
	equal(t, v.Requests()[0].Method, "GET")
	equal(t, v.Requests()[0].URI, "/backend-api/wham/usage")
	equal(t, v.Requests()[0].Header("Authorization"), "Bearer "+freshToken(t))
	equal(t, v.Requests()[0].Header("Chatgpt-Account-Id"), "acct-1")
}
func TestUsageProbeRefusal(t *testing.T) {
	v, _, p := setup(t)
	v.RespondAt("/backend-api/wham/usage", answer(403, `{"error":"forbidden"}`))
	_, err := p.Quota().Probe(t.Context())
	var quota *provider.QuotaError
	if !errors.As(err, &quota) {
		t.Fatalf("error: %v", err)
	}
	equal(t, p.Quota().Latest(), (*core.QuotaSnapshot)(nil))
	equal(t, quota.Kind, provider.QuotaUnavailable)
	equal(t, quota.Message, "Codex usage request failed with HTTP 403")
	equal(t, len(v.Requests()), 1)
}
