package codex

import (
	"context"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"golang.org/x/text/cases"
	"golang.org/x/text/language"
)

//demi:wire open
type usageStatus struct {
	PlanType  *string    `json:"plan_type,omitzero" check:"nullabsent"`
	RateLimit *rateLimit `json:"rate_limit,omitzero" check:"nullabsent"`
}

//demi:wire open
type rateLimit struct {
	Primary   *usageWindow `json:"primary_window,omitzero" check:"nullabsent"`
	Secondary *usageWindow `json:"secondary_window,omitzero" check:"nullabsent"`
}

//demi:wire open
type usageWindow struct {
	UsedPercent   float64 `json:"used_percent"`
	WindowSeconds float64 `json:"limit_window_seconds"`
	ResetAt       float64 `json:"reset_at"`
}
type quotaSource struct{ provider *Provider }

func (*quotaSource) ProbeCost() *provider.ProbeCost { return new(provider.ProbeFree) }
func (q *quotaSource) Probe(ctx context.Context) (provider.ProbeReading, error) {
	response, err := q.provider.get(ctx, q.provider.usageURL, false)
	if err != nil {
		var failure *provider.AuthFailure
		if errors.As(err, &failure) {
			return provider.ProbeReading{}, failure.QuotaError()
		}
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: err.Error()}
	}
	defer response.Body.Close()
	stored, failure := q.provider.auth.stored(ctx)
	if failure != nil {
		return provider.ProbeReading{}, failure.QuotaError()
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: fmt.Sprintf("Codex usage request failed with HTTP %d", response.StatusCode)}
	}
	raw, err := io.ReadAll(response.Body)
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: provider.TransportFailure("Codex usage", err).Message}
	}
	usage, err := decode[usageStatus](raw)
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaInvalid, Message: fmt.Sprintf("Codex usage status cannot be read: %v", err)}
	}
	reading := provider.ProbeReading{AccountLabel: new(stored.label().Label), Windows: []core.QuotaWindow{}}
	if usage.PlanType != nil && *usage.PlanType != "" {
		words := strings.ReplaceAll(*usage.PlanType, "_", " ")
		first, size := utf8.DecodeRuneInString(words)
		label := cases.Upper(language.Und).String(string(first)) + words[size:]
		reading.Plan = &core.QuotaPlan{ID: *usage.PlanType, Label: label}
	}
	if usage.RateLimit != nil {
		for i, window := range []*usageWindow{usage.RateLimit.Primary, usage.RateLimit.Secondary} {
			if window == nil {
				continue
			}
			reading.Windows = append(reading.Windows, quotaWindow(i, provider.ClampUsedPercent(window.UsedPercent), new(window.WindowSeconds/60), provider.UnixSeconds(window.ResetAt)))
		}
	}
	return reading, nil
}
func (*quotaSource) Observe(observation provider.Observation) ([]core.QuotaWindow, bool) {
	if observation.Headers == nil {
		return nil, false
	}
	windows := []core.QuotaWindow{}
	for i, id := range []string{"primary", "secondary"} {
		prefix := "x-codex-" + id
		if _, ok := observation.Headers[http.CanonicalHeaderKey(prefix+"-used-percent")]; !ok {
			continue
		}
		used := provider.HeaderNumber(observation.Headers, prefix+"-used-percent")
		if used != nil {
			used = provider.ClampUsedPercent(*used)
		}
		var reset *core.Timestamp
		if seconds := provider.HeaderNumber(observation.Headers, prefix+"-reset-at"); seconds != nil {
			reset = provider.UnixSeconds(*seconds)
		}
		windows = append(windows, quotaWindow(i, used, provider.HeaderNumber(observation.Headers, prefix+"-window-minutes"), reset))
	}
	return windows, len(windows) > 0
}
func quotaWindow(index int, used, minutes *float64, reset *core.Timestamp) core.QuotaWindow {
	id, label := "primary", "Primary"
	if index == 1 {
		id, label = "secondary", "Secondary"
	}
	if minutes != nil && *minutes > 0 {
		n := *minutes
		switch {
		case math.Mod(n, 10080) == 0:
			if n == 10080 {
				label = "Weekly"
			} else {
				label = fmt.Sprintf("%g-week", n/10080)
			}
		case math.Mod(n, 1440) == 0:
			if n == 1440 {
				label = "Daily"
			} else {
				label = fmt.Sprintf("%g-day", n/1440)
			}
		case math.Mod(n, 60) == 0:
			label = fmt.Sprintf("%g-hour", n/60)
		default:
			label = fmt.Sprintf("%g-minute", n)
		}
	}
	return core.QuotaWindow{ID: id, Label: label, UsedPercent: used, Unit: new(core.QuotaUnitPercent), ResetsAt: reset, Severity: provider.Severity(used)}
}
