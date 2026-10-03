package grokbuild

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"sync"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

type quotaSource struct {
	auth                *auth
	http                *http.Client
	userURL, billingURL *url.URL
}

// ProbeCost reports the cost of a quota probe.
func (*quotaSource) ProbeCost() (provider.ProbeCost, bool) {
	return provider.ProbeFree, true
}

// Probe fetches the account quota windows.
func (q *quotaSource) Probe(ctx context.Context) (provider.ProbeReading, error) {
	s, failure := q.auth.credentials(ctx, q.http, nil)
	if failure != nil {
		return provider.ProbeReading{}, failure
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	type result struct {
		text    string
		err     error
		billing bool
	}
	results := make(chan result, 2)
	var workers sync.WaitGroup
	defer workers.Wait()
	for i, u := range []*url.URL{q.userURL, q.billingURL} {
		workers.Go(func() {
			text, err := q.fetch(ctx, u, s)
			results <- result{text: text, err: err, billing: i == 1}
		})
	}
	var user, billing string
	var failureErr error
	for range 2 {
		result := <-results
		if result.err != nil && failureErr == nil {
			failureErr = result.err
			cancel()
		}
		if result.billing {
			billing = result.text
		} else {
			user = result.text
		}
	}
	if failureErr != nil {
		return provider.ProbeReading{}, failureErr
	}
	return quotaReading(user, billing, s.Email), nil
}

//nolint:staticcheck // ST1005: user-facing error text starts with a capital letter.
func (q *quotaSource) fetch(ctx context.Context, u *url.URL, s secret) (string, error) {
	failed := func(err error) error {
		return fmt.Errorf("Grok quota request failed: %v", provider.WithoutURL(err))
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u.String(), nil)
	if err != nil {
		return "", failed(err)
	}
	req.Header = identityHeaders(s)
	req.Header.Set("Accept", "application/json")
	response, err := q.http.Do(req)
	if err != nil {
		return "", failed(err)
	}
	// The response is consumed or abandoned; close errors cannot change its result.
	defer func() { _ = response.Body.Close() }()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return "", failed(err)
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		text := []rune(string(body))
		if len(text) > 200 {
			text = text[:200]
		}
		return "", fmt.Errorf("Grok quota request failed (%d): %s", response.StatusCode, string(text))
	}
	return string(body), nil
}

type quotaUser struct {
	Tier  provider.ReportedString `json:"subscriptionTier" wire:"optional"`
	Email provider.ReportedString `json:"email"            wire:"optional"`
}
type billingAnswer struct {
	Config provider.Reported[billingConfig] `json:"config" wire:"optional"`
}
type billingConfig struct {
	Percent provider.Reported[float64]       `json:"creditUsagePercent" wire:"optional"`
	Period  provider.Reported[billingPeriod] `json:"currentPeriod"      wire:"optional"`
	End     provider.ReportedString          `json:"billingPeriodEnd"   wire:"optional"`
	Limit   provider.Reported[amount]        `json:"monthlyLimit"       wire:"optional"`
	Used    provider.Reported[amount]        `json:"used"               wire:"optional"`
	Cap     provider.Reported[amount]        `json:"onDemandCap"        wire:"optional"`
}
type billingPeriod struct {
	Kind provider.ReportedString `json:"type" wire:"optional"`
	End  provider.ReportedString `json:"end"  wire:"optional"`
}
type amount float64

// UnmarshalJSON reads the vendor value used by this provider.
func (a *amount) UnmarshalJSON(data []byte) error {
	n, err := provider.DecodeUntagged[float64](string(data))
	if err != nil {
		v, e := provider.DecodeUntagged[struct {
			Val float64 `json:"val"`
		}](string(data))
		if e != nil {
			return e
		}
		n = v.Val
	}
	*a = amount(n)
	return nil
}

func quotaReading(userText, billingText string, email *string) provider.ProbeReading {
	user, _ := provider.DecodeUntagged[quotaUser](userText)
	billing, _ := provider.DecodeUntagged[billingAnswer](billingText)
	var config billingConfig
	if billing.Config.Value != nil {
		config = *billing.Config.Value
	}
	var period billingPeriod
	if config.Period.Value != nil {
		period = *config.Period.Value
	}
	end := period.End.Value
	if end == nil {
		end = config.End.Value
	}
	var resets *core.Timestamp
	if end != nil {
		resets = provider.RFC3339(*end)
	}
	var used, limit, percent *float64
	if config.Used.Value != nil {
		v := float64(*config.Used.Value)
		used = &v
	}
	if config.Limit.Value != nil {
		v := float64(*config.Limit.Value)
		limit = &v
	}
	if config.Percent.Value != nil {
		percent = provider.ClampUsedPercent(*config.Percent.Value)
	}
	if percent == nil && used != nil && limit != nil {
		percent = provider.UsedPercentFromRatio(*used, *limit)
	}
	reading := provider.ProbeReading{AccountLabel: email, Windows: []core.QuotaWindow{}}
	if email == nil {
		reading.AccountLabel = user.Email.Value
	}
	if user.Tier.Value != nil && *user.Tier.Value != "" {
		reading.Plan = &core.QuotaPlan{ID: *user.Tier.Value, Label: *user.Tier.Value}
	}
	reading.Windows = quotaWindows(config, period, resets, used, limit, percent)
	return reading
}

// Observe reads quota windows from a provider observation.
func (*quotaSource) Observe(observation provider.Observation) []core.QuotaWindow {
	switch o := observation.(type) {
	case *provider.CLIObservation:
		return nil
	case *provider.HTTPObservation:
		var windows []core.QuotaWindow
		for _, w := range []struct {
			id, label, suffix string
			unit              core.QuotaUnit
		}{{"rpm", "Requests (short window)", "requests", "requests"}, {"tpm", "Tokens (short window)", "tokens", "tokens"}} {
			limit := provider.HeaderNumber(o.Headers, "x-ratelimit-limit-"+w.suffix)
			if limit == nil {
				continue
			}
			remaining := provider.HeaderNumber(o.Headers, "x-ratelimit-remaining-"+w.suffix)
			var used, percent *float64
			if remaining != nil {
				v := *limit - *remaining
				used = &v
				percent = provider.UsedPercentFromRatio(v, *limit)
			}
			windows = append(
				windows,
				core.QuotaWindow{
					ID:          w.id,
					Label:       w.label,
					Limit:       limit,
					Used:        used,
					UsedPercent: percent,
					Unit:        &w.unit,
					Severity:    provider.Severity(percent),
				},
			)
		}
		return windows
	}
	return nil
}

func quotaWindows(
	config billingConfig,
	period billingPeriod,
	resets *core.Timestamp,
	used, limit, percent *float64,
) []core.QuotaWindow {
	windows := []core.QuotaWindow{}
	credits := core.QuotaUnit("credits")
	if used != nil || limit != nil || percent != nil {
		id, name := "monthly", "Monthly credits"
		if period.Kind.Value != nil && *period.Kind.Value == "USAGE_PERIOD_TYPE_WEEKLY" {
			id, name = "weekly", "Weekly credits"
		}
		windows = append(
			windows,
			core.QuotaWindow{
				ID:          id,
				Label:       name,
				Used:        used,
				Limit:       limit,
				UsedPercent: percent,
				Unit:        &credits,
				ResetsAt:    resets,
				Severity:    provider.Severity(percent),
			},
		)
	}
	if config.Cap.Value != nil && *config.Cap.Value > 0 {
		capWindow := float64(*config.Cap.Value)
		windows = append(
			windows,
			core.QuotaWindow{
				ID:       "on_demand_cap",
				Label:    "On-demand cap",
				Limit:    &capWindow,
				Unit:     &credits,
				ResetsAt: resets,
			},
		)
	}
	return windows
}
