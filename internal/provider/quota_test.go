package provider_test

import (
	"context"
	"errors"
	"math"
	"net/http"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

type quotaSource struct {
	cost         *provider.ProbeCost
	probes       []provider.ProbeReading
	observations [][]types.QuotaWindow
}

func (s *quotaSource) ProbeCost() (provider.ProbeCost, bool) {
	if s.cost == nil {
		return 0, false
	}
	return *s.cost, true
}

func (s *quotaSource) Probe(context.Context) (provider.ProbeReading, error) {
	next := s.probes[0]
	s.probes = s.probes[1:]
	return next, nil
}

func (s *quotaSource) Observe(provider.Observation) []types.QuotaWindow {
	next := s.observations[0]
	s.observations = s.observations[1:]
	return next
}

func quotaWindow(id string, used float64) types.QuotaWindow {
	unit := types.QuotaUnit("percent")
	return types.QuotaWindow{ID: id, Label: id, UsedPercent: &used, Unit: &unit, Severity: provider.Severity(&used)}
}

func TestQuotaObservationKeepsPlan(t *testing.T) {
	free := provider.ProbeFree
	label := "person@example.com"
	plan := &types.QuotaPlan{ID: "pro", Label: "pro"}
	source := &quotaSource{
		cost: &free,
		probes: []provider.ProbeReading{
			{
				Plan:         plan,
				AccountLabel: &label,
				Windows:      []types.QuotaWindow{quotaWindow("monthly", 25), quotaWindow("rpm", 10)},
			},
		},
		observations: [][]types.QuotaWindow{{quotaWindow("rpm", 40), quotaWindow("tpm", 5)}},
	}
	quota := provider.NewQuota(source, &provider.MemorySnapshots{}, providertest.FixedClock(now))
	if quota.Latest() != nil {
		t.Fatal("unexpected initial snapshot")
	}
	probed, err := quota.Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, probed.Source, types.SnapshotSource("probe"))
	requireEqual(t, probed.ObservedAt, now)
	quota.Observe(&provider.HTTPObservation{Status: 200})
	observed := quota.Latest()
	requireEqual(t, observed.Source, types.SnapshotSource("observation"))
	requireEqual(t, observed.Plan, plan)
	requireEqual(t, *observed.AccountLabel, label)
	requireEqual(
		t,
		observed.Windows,
		[]types.QuotaWindow{quotaWindow("monthly", 25), quotaWindow("rpm", 40), quotaWindow("tpm", 5)},
	)
	// A caller's changes cannot alter a kept snapshot or an earlier returned one.
	*observed.Windows[0].UsedPercent = 99
	requireEqual(t, *quota.Latest().Windows[0].UsedPercent, 25.0)
	requireEqual(t, *probed.Windows[1].UsedPercent, 10.0)
}

func TestQuotaProbeKeepsUnnamedWindows(t *testing.T) {
	free := provider.ProbeFree
	plan := &types.QuotaPlan{ID: "pro", Label: "pro"}
	source := &quotaSource{
		cost: &free,
		probes: []provider.ProbeReading{
			{Plan: plan, Windows: []types.QuotaWindow{quotaWindow("weekly", 40)}},
			{},
		},
		observations: [][]types.QuotaWindow{{quotaWindow("requests", 5)}, nil},
	}
	quota := provider.NewQuota(source, &provider.MemorySnapshots{}, providertest.FixedClock(now))
	quota.Observe(&provider.HTTPObservation{Status: 200})
	probed, err := quota.Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, probed.Plan, plan)
	requireEqual(t, probed.Windows, []types.QuotaWindow{quotaWindow("requests", 5), quotaWindow("weekly", 40)})
	quota.Observe(&provider.HTTPObservation{Status: 200})
	requireEqual(t, quota.Latest(), probed)
	again, err := quota.Probe(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if again.Plan != nil || len(again.Windows) != 2 {
		t.Fatalf("%+v", again)
	}
}

func TestQuotaNeverSpendsInference(t *testing.T) {
	inference := provider.ProbeInference
	for _, tc := range []struct {
		cost *provider.ProbeCost
		want error
	}{{nil, provider.ErrQuotaUnsupported}, {&inference, provider.ErrQuotaRequiresInference}} {
		quota := provider.NewQuota(
			&quotaSource{cost: tc.cost},
			&provider.MemorySnapshots{},
			providertest.FixedClock(now),
		)
		_, err := quota.Probe(t.Context())
		if !errors.Is(err, tc.want) {
			t.Fatalf("%v", err)
		}
		if quota.Latest() != nil {
			t.Fatal("probe created snapshot")
		}
	}
}

func TestQuotaUnitsAndResetTimes(t *testing.T) {
	for _, tc := range []struct{ value, want float64 }{{150, 100}, {-1, 0}, {35.5, 35.5}} {
		requireEqual(t, *provider.ClampUsedPercent(tc.value), tc.want)
	}
	if provider.ClampUsedPercent(math.NaN()) != nil {
		t.Fatal("NaN share")
	}
	requireEqual(t, *provider.UsedPercentFromRatio(25, 100), 25.0)
	if provider.UsedPercentFromRatio(1, 0) != nil {
		t.Fatal("zero limit")
	}
	for _, tc := range []struct {
		share float64
		want  types.QuotaSeverity
	}{{50, "normal"}, {80, "warning"}, {90, "warning"}, {95, "critical"}} {
		requireEqual(t, *provider.Severity(&tc.share), tc.want)
	}
	if provider.Severity(nil) != nil {
		t.Fatal("unknown severity")
	}
	requireEqual(t, *provider.UnixSeconds(1700000000), types.Timestamp("2023-11-14T22:13:20.000Z"))
	requireEqual(t, *provider.UnixSeconds(1700000000.5), types.Timestamp("2023-11-14T22:13:20.500Z"))
	if provider.UnixSeconds(1e20) != nil || provider.UnixSeconds(math.NaN()) != nil {
		t.Fatal("invalid reset time")
	}
	requireEqual(t, *provider.RFC3339("2026-08-21T10:45:24.951512+00:00"), types.Timestamp("2026-08-21T10:45:24.951Z"))
	requireEqual(t, *provider.RFC3339("2026-08-01T00:00:00Z"), types.Timestamp("2026-08-01T00:00:00.000Z"))
	if provider.RFC3339("soon") != nil || provider.RFC3339("1790062659") != nil {
		t.Fatal("invalid RFC3339")
	}
	headers := http.Header{}
	for name, value := range map[string]string{"x-used": " 35.5 ", "x-word": "full", "x-blank": "", "x-infinite": "inf"} {
		headers.Set(name, value)
	}
	requireEqual(t, *provider.HeaderNumber(headers, "x-used"), 35.5)
	for _, name := range []string{"x-word", "x-blank", "x-infinite", "x-absent"} {
		if provider.HeaderNumber(headers, name) != nil {
			t.Fatalf("accepted %s", name)
		}
	}
}
