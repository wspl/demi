package grokbuild

import (
	"sort"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider/providertest"
)

func probe(t *testing.T, user, billing string) (*core.QuotaSnapshot, *providertest.MockVendor) {
	t.Helper()
	v := providertest.StartVendor(t)
	v.RespondAt("/v1/user", answer(200, user))
	v.RespondAt("/v1/billing", answer(200, billing))
	p, _ := fixture(t, v, map[string]any{"email": nil})
	reading, err := p.Quota().Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return reading, v
}
func TestSubscriptionAndBillingProbe(t *testing.T) {
	s, v := probe(t, `{"subscriptionTier":"XPremiumPlus","email":"a@b.com","hasGrokCodeAccess":true}`, `{"config":{"monthlyLimit":{"val":20000},"used":{"val":5000},"onDemandCap":{"val":0},"billingPeriodEnd":"2026-08-01T00:00:00+00:00"}}`)
	requests := v.Requests()
	paths := []string{requests[0].URI, requests[1].URI}
	sort.Strings(paths)
	equal(t, []string{"/v1/billing?format=credits", "/v1/user?include=subscription"}, paths)
	equal(t, &core.QuotaPlan{ID: "XPremiumPlus", Label: "XPremiumPlus"}, s.Plan)
	equal(t, "a@b.com", *s.AccountLabel)
	equal(t, 1, len(s.Windows))
	w := s.Windows[0]
	equal(t, "monthly", w.ID)
	equal(t, "Monthly credits", w.Label)
	equal(t, 25.0, *w.UsedPercent)
	equal(t, 5000.0, *w.Used)
	equal(t, 20000.0, *w.Limit)
	equal(t, core.QuotaUnit("credits"), *w.Unit)
	equal(t, core.Timestamp("2026-08-01T00:00:00.000Z"), *w.ResetsAt)
	equal(t, core.QuotaSeverity("normal"), *w.Severity)
}
func TestWeeklyCreditsAndCap(t *testing.T) {
	s, _ := probe(t, `{"subscriptionTier":"XPremiumPlus"}`, `{"config":{"creditUsagePercent":2,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-08-14T10:45:24.951512+00:00","end":"2026-08-21T10:45:24.951512+00:00"},"onDemandCap":50,"billingPeriodEnd":"2026-08-30T00:00:00+00:00"}}`)
	equal(t, 2, len(s.Windows))
	weekly, capWindow := s.Windows[0], s.Windows[1]
	equal(t, "weekly", weekly.ID)
	equal(t, "Weekly credits", weekly.Label)
	equal(t, 2.0, *weekly.UsedPercent)
	if weekly.Used != nil || weekly.Limit != nil {
		t.Fatal("invented credit amounts")
	}
	equal(t, core.Timestamp("2026-08-21T10:45:24.951Z"), *weekly.ResetsAt)
	equal(t, "on_demand_cap", capWindow.ID)
	equal(t, "On-demand cap", capWindow.Label)
	equal(t, 50.0, *capWindow.Limit)
	if capWindow.UsedPercent != nil {
		t.Fatal("invented cap percentage")
	}
}
func TestUnmeteredAndOddQuotaFields(t *testing.T) {
	s, _ := probe(t, `{"subscriptionTier":"XPremium","email":"u@example.com"}`, `{"config":{"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-09-18T00:00:00Z","end":"2026-09-25T00:00:00Z"},"onDemandUsed":{"val":0},"prepaidBalance":{"val":0},"monthlyLimit":"lots","billingPeriodEnd":"2026-10-01T00:00:00Z"}}`)
	equal(t, "XPremium", s.Plan.Label)
	equal(t, 0, len(s.Windows))
}
func TestEveryChatObservesLimits(t *testing.T) {
	v := providertest.StartVendor(t)
	for i, remaining := range []string{"100", "0"} {
		response := chat()
		if i == 1 {
			response = answer(429, "")
		}
		response.Headers.Set("x-ratelimit-limit-requests", "120")
		response.Headers.Set("x-ratelimit-remaining-requests", remaining)
		response.Headers.Set("x-ratelimit-limit-tokens", "5000")
		response.Headers.Set("x-ratelimit-remaining-tokens", "4000")
		v.RespondAt(chatPath, response)
	}
	p, _ := fixture(t, v, nil)
	for _, used := range []float64{20, 120} {
		run(t, p, providertest.InferenceRequest())
		s := p.Quota().Latest()
		equal(t, 2, len(s.Windows))
		var rpm, tpm core.QuotaWindow
		for _, w := range s.Windows {
			switch w.ID {
			case "rpm":
				rpm = w
			case "tpm":
				tpm = w
			}
		}
		equal(t, used, *rpm.Used)
		equal(t, 120.0, *rpm.Limit)
		equal(t, 1000.0, *tpm.Used)
		equal(t, 5000.0, *tpm.Limit)
	}
	equal(t, core.QuotaSeverity("critical"), *p.Quota().Latest().Windows[0].Severity)
}
