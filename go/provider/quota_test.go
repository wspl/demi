package provider_test

import (
	"context"
	"errors"
	"math"
	"net/http"
	"reflect"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
)

type quotaSource struct {
	cost         *provider.ProbeCost
	probes       []provider.ProbeReading
	observations [][]core.QuotaWindow
}

func (s *quotaSource) ProbeCost() *provider.ProbeCost { return s.cost }
func (s *quotaSource) Probe(context.Context) (provider.ProbeReading, error) {
	r := s.probes[0]
	s.probes = s.probes[1:]
	return r, nil
}
func (s *quotaSource) Observe(provider.Observation) ([]core.QuotaWindow, bool) {
	w := s.observations[0]
	s.observations = s.observations[1:]
	return w, w != nil
}
func TestQuotaProbeAndObservationMerge(t *testing.T) {
	cost := provider.ProbeFree
	label := "person@example.invalid"
	source := &quotaSource{cost: &cost, probes: []provider.ProbeReading{{Plan: &core.QuotaPlan{ID: "pro", Label: "Pro"}, AccountLabel: &label, Windows: []core.QuotaWindow{{ID: "monthly", Label: "monthly"}, {ID: "rpm", Label: "old"}}}, {}}, observations: [][]core.QuotaWindow{{{ID: "rpm", Label: "new"}, {ID: "tpm", Label: "tokens"}}, nil}}
	quota := provider.NewProviderQuota(source, &provider.MemorySnapshots{}, providertest.FixedClock{})
	if quota.Latest() != nil {
		t.Fatal("initial snapshot")
	}
	probed, err := quota.Probe(t.Context())
	if err != nil || probed.Source != core.SnapshotSourceProbe {
		t.Fatal(probed, err)
	}
	quota.Observe(provider.Observation{})
	observed := quota.Latest()
	if observed.Source != core.SnapshotSourceObservation || observed.Plan.ID != "pro" || *observed.AccountLabel != label {
		t.Fatal(observed)
	}
	want := []core.QuotaWindow{{ID: "monthly", Label: "monthly"}, {ID: "rpm", Label: "new"}, {ID: "tpm", Label: "tokens"}}
	if !reflect.DeepEqual(observed.Windows, want) {
		t.Fatal(observed.Windows)
	}
	if probed.Windows[1].Label != "old" {
		t.Fatal("merge modified previous snapshot")
	}
	quota.Observe(provider.Observation{})
	if quota.Latest() != observed {
		t.Fatal("unreadable observation modified snapshot")
	}
	cleared, err := quota.Probe(t.Context())
	if err != nil || cleared.Plan != nil || cleared.AccountLabel != nil || !reflect.DeepEqual(cleared.Windows, want) {
		t.Fatal(cleared, err)
	}
	for _, cost := range []*provider.ProbeCost{nil, new(provider.ProbeInference)} {
		quota := provider.NewProviderQuota(&quotaSource{cost: cost}, &provider.MemorySnapshots{}, providertest.FixedClock{})
		_, err := quota.Probe(t.Context())
		want := provider.ErrQuotaUnsupported
		if cost != nil {
			want = provider.ErrQuotaRequiresInference
		}
		if !errors.Is(err, want) || quota.Latest() != nil {
			t.Fatal(err)
		}
	}
}
func TestQuotaUnitsAndUnknownValues(t *testing.T) {
	for _, tc := range []struct{ in, want float64 }{{150, 100}, {-1, 0}, {35.5, 35.5}} {
		if got := provider.ClampUsedPercent(tc.in); got == nil || *got != tc.want {
			t.Fatal(tc, got)
		}
	}
	if provider.ClampUsedPercent(math.NaN()) != nil || provider.UsedPercentFromRatio(1, 0) != nil {
		t.Fatal("invalid share accepted")
	}
	if got := provider.UsedPercentFromRatio(25, 100); got == nil || *got != 25 {
		t.Fatal(got)
	}
	for _, tc := range []struct {
		in   float64
		want core.QuotaSeverity
	}{{50, core.QuotaSeverityNormal}, {80, core.QuotaSeverityWarning}, {90, core.QuotaSeverityWarning}, {95, core.QuotaSeverityCritical}} {
		if got := provider.Severity(&tc.in); got == nil || *got != tc.want {
			t.Fatal(tc, got)
		}
	}
	if provider.Severity(nil) != nil {
		t.Fatal("unknown severity")
	}
	if got := provider.UnixSeconds(1700000000.5); got == nil || got.String() != "2023-11-14T22:13:20.500Z" {
		t.Fatal(got)
	}
	if provider.UnixSeconds(1e20) != nil || provider.UnixSeconds(math.NaN()) != nil {
		t.Fatal("invalid time")
	}
	if got := provider.RFC3339("2026-08-21T10:45:24.951512+00:00"); got == nil || got.String() != "2026-08-21T10:45:24.951Z" {
		t.Fatal(got)
	}
	if provider.RFC3339("soon") != nil || provider.RFC3339("1790062659") != nil {
		t.Fatal("invalid time text")
	}
	headers := http.Header{"X-Used": {" 35.5 "}, "X-Word": {"full"}, "X-Blank": {""}, "X-Infinite": {"inf"}}
	if got := provider.HeaderNumber(headers, "x-used"); got == nil || *got != 35.5 {
		t.Fatal(got)
	}
	for _, name := range []string{"x-word", "x-blank", "x-infinite", "x-absent"} {
		if provider.HeaderNumber(headers, name) != nil {
			t.Fatal(name)
		}
	}
}
