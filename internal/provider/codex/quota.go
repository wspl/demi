package codex

import (
	"context"
	"fmt"
	"io"
	"math"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

type quotaSource struct{ p *Provider }

// ProbeCost reports the cost of a quota probe.
func (*quotaSource) ProbeCost() (provider.ProbeCost, bool) {
	return provider.ProbeFree, true
}

type usageStatus struct {
	Plan  *string    `json:"plan_type"`
	Limit *rateLimit `json:"rate_limit"`
}
type rateLimit struct {
	Primary   *usageWindow `json:"primary_window"`
	Secondary *usageWindow `json:"secondary_window"`
}
type usageWindow struct {
	Used    float64 `json:"used_percent"`
	Seconds float64 `json:"limit_window_seconds"`
	Reset   float64 `json:"reset_at"`
}

// Probe fetches the account quota windows.
//
//nolint:staticcheck // ST1005: user-facing error text starts with a capital letter.
func (q *quotaSource) Probe(ctx context.Context) (provider.ProbeReading, error) {
	s, err := q.p.credentials(ctx, q.p.http, nil)
	if err != nil {
		return provider.ProbeReading{}, err
	}
	stored, err := q.p.stored(ctx)
	if err != nil {
		return provider.ProbeReading{}, err
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, q.p.usageURL, nil)
	if err != nil {
		return provider.ProbeReading{}, err
	}
	request.Header = accountHeaders(s)
	request.Header.Set("Accept", "application/json")
	response, err := q.p.http.Do(request)
	if err != nil {
		return provider.ProbeReading{}, fmt.Errorf("Codex usage request failed: %v", provider.WithoutURL(err))
	}
	defer func() {
		_ = response.Body.Close()
	}() // The reader reports IO failures; close releases the response.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return provider.ProbeReading{}, fmt.Errorf("Codex usage request failed with HTTP %d", response.StatusCode)
	}
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return provider.ProbeReading{}, fmt.Errorf("Codex usage request failed: %v", err)
	}
	usage, err := provider.DecodeUntagged[usageStatus](string(data))
	if err != nil {
		return provider.ProbeReading{}, fmt.Errorf("Codex usage status cannot be read: %v", err)
	}
	label := stored.label().Label
	result := provider.ProbeReading{AccountLabel: &label, Windows: []types.QuotaWindow{}}
	if usage.Plan != nil && *usage.Plan != "" {
		result.Plan = &types.QuotaPlan{ID: *usage.Plan, Label: planLabel(*usage.Plan)}
	}
	if usage.Limit != nil {
		for index, w := range []*usageWindow{usage.Limit.Primary, usage.Limit.Secondary} {
			if w != nil {
				minutes := w.Seconds / 60
				result.Windows = append(
					result.Windows,
					windowOf(index, provider.ClampUsedPercent(w.Used), &minutes, provider.UnixSeconds(w.Reset)),
				)
			}
		}
	}
	return result, nil
}

// Observe reads quota windows from a provider observation.
func (*quotaSource) Observe(observation provider.Observation) []types.QuotaWindow {
	var headers http.Header
	switch o := observation.(type) {
	case *provider.HTTPObservation:
		headers = o.Headers
	case *provider.CLIObservation:
		return nil
	}
	var windows []types.QuotaWindow
	for index, kind := range []string{"primary", "secondary"} {
		prefix := "x-codex-" + kind
		if _, present := headers[http.CanonicalHeaderKey(prefix+"-used-percent")]; !present {
			continue
		}
		var used *float64
		if v := provider.HeaderNumber(headers, prefix+"-used-percent"); v != nil {
			used = provider.ClampUsedPercent(*v)
		}
		var reset *types.Timestamp
		if v := provider.HeaderNumber(headers, prefix+"-reset-at"); v != nil {
			reset = provider.UnixSeconds(*v)
		}
		windows = append(
			windows,
			windowOf(index, used, provider.HeaderNumber(headers, prefix+"-window-minutes"), reset),
		)
	}
	return windows
}

func windowOf(index int, used, minutes *float64, reset *types.Timestamp) types.QuotaWindow {
	id, label := "primary", "Primary"
	if index == 1 {
		id, label = "secondary", "Secondary"
	}
	if minutes != nil && *minutes > 0 {
		m := *minutes
		switch {
		case m == 10080:
			label = "Weekly"
		case math.Mod(m, 10080) == 0:
			label = fmt.Sprintf("%g-week", m/10080)
		case m == 1440:
			label = "Daily"
		case math.Mod(m, 1440) == 0:
			label = fmt.Sprintf("%g-day", m/1440)
		case math.Mod(m, 60) == 0:
			label = fmt.Sprintf("%g-hour", m/60)
		default:
			label = fmt.Sprintf("%g-minute", m)
		}
	}
	unit := types.QuotaUnit("percent")
	return types.QuotaWindow{
		ID:          id,
		Label:       label,
		UsedPercent: used,
		ResetsAt:    reset,
		Unit:        &unit,
		Severity:    provider.Severity(used),
	}
}

func planLabel(plan string) string {
	words := []rune(strings.ReplaceAll(plan, "_", " "))
	if len(words) == 0 {
		return ""
	}
	return strings.ToUpper(string(words[0])) + string(words[1:])
}
