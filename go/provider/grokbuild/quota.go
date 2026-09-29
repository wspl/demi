package grokbuild

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"math"
	"net/http"
	"net/url"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire open
type quotaUser struct {
	SubscriptionTier *provider.ReportedString `json:"subscriptionTier,omitzero" check:"nullabsent,func=provider.Validate"`
	Email            *provider.ReportedString `json:"email,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:wire open
type billing struct {
	Config *reportedConfig `json:"config,omitzero" check:"nullabsent"`
}

//demi:wire open
type billingConfig struct {
	CreditUsagePercent *provider.ReportedNumber `json:"creditUsagePercent,omitzero" check:"nullabsent,func=provider.Validate"`
	CurrentPeriod      *reportedPeriod          `json:"currentPeriod,omitzero" check:"nullabsent"`
	BillingPeriodEnd   *provider.ReportedString `json:"billingPeriodEnd,omitzero" check:"nullabsent,func=provider.Validate"`
	MonthlyLimit       *amount                  `json:"monthlyLimit,omitzero" check:"nullabsent"`
	Used               *amount                  `json:"used,omitzero" check:"nullabsent"`
	OnDemandCap        *amount                  `json:"onDemandCap,omitzero" check:"nullabsent"`
}

//demi:wire open
type period struct {
	Type *provider.ReportedString `json:"type,omitzero" check:"nullabsent,func=provider.Validate"`
	End  *provider.ReportedString `json:"end,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:opaque
type reportedConfig struct{ value billingConfig }

func (v *reportedConfig) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.value, _ = decode[billingConfig](raw)
	return nil
}

//demi:opaque
type reportedPeriod struct{ value period }

func (v *reportedPeriod) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.value, _ = decode[period](raw)
	return nil
}

//demi:opaque
type amount struct{ value *float64 }

func (v *amount) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.value = nil
	if raw.Kind() == '{' {
		var object map[string]jsontext.Value
		if json.Unmarshal(raw, &object) != nil {
			return nil
		}
		raw = object["val"]
	}
	var value float64
	if raw.Kind() == '0' && json.Unmarshal(raw, &value) == nil && !math.IsInf(value, 0) && !math.IsNaN(value) {
		v.value = &value
	}
	return nil
}
func amountValue(a *amount) *float64 {
	if a == nil {
		return nil
	}
	return a.value
}

type quotaSource struct{ p *Provider }

func (*quotaSource) ProbeCost() *provider.ProbeCost { return new(provider.ProbeFree) }
func (q *quotaSource) Probe(ctx context.Context) (provider.ProbeReading, error) {
	secret, failure := q.p.credentials(ctx, q.p.http, nil)
	if failure != nil {
		return provider.ProbeReading{}, failure.QuotaError()
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	type result struct {
		raw     []byte
		err     error
		billing bool
	}
	results := make(chan result, 2)
	for _, isBilling := range []bool{false, true} {
		go func() {
			endpoint, _ := url.Parse(q.p.userURL)
			query := "include=subscription"
			if isBilling {
				endpoint, _ = url.Parse(q.p.billingURL)
				query = "format=credits"
			}
			endpoint.RawQuery = query
			headers := identityHeaders(secret)
			headers.Set("Accept", "application/json")
			response, err := send(ctx, q.p.http, http.MethodGet, endpoint.String(), headers, nil)
			var raw []byte
			if err == nil {
				raw, err = readResponse(response)
				if err == nil && (response.StatusCode < 200 || response.StatusCode >= 300) {
					text := []rune(string(raw))
					if len(text) > 200 {
						text = text[:200]
					}
					err = fmt.Errorf("Grok quota request failed (%d): %s", response.StatusCode, string(text))
				}
			} else {
				err = provider.TransportFailure("Grok quota", err)
			}
			results <- result{raw: raw, err: err, billing: isBilling}
		}()
	}
	var userRaw, billingRaw []byte
	var failed error
	for range 2 {
		result := <-results
		if result.err != nil && failed == nil {
			failed = result.err
			cancel()
		}
		if result.billing {
			billingRaw = result.raw
		} else {
			userRaw = result.raw
		}
	}
	if failed != nil {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: failed.Error()}
	}
	user, _ := decode[quotaUser](userRaw)
	bill, _ := decode[billing](billingRaw)
	config := billingConfig{}
	if bill.Config != nil {
		config = bill.Config.value
	}
	period := period{}
	if config.CurrentPeriod != nil {
		period = config.CurrentPeriod.value
	}
	end := reported(period.End)
	if end == nil {
		end = reported(config.BillingPeriodEnd)
	}
	var reset *core.Timestamp
	if end != nil {
		reset = provider.RFC3339(*end)
	}
	limit, used := amountValue(config.MonthlyLimit), amountValue(config.Used)
	var percent *float64
	if config.CreditUsagePercent != nil && config.CreditUsagePercent.Value != nil {
		percent = provider.ClampUsedPercent(*config.CreditUsagePercent.Value)
	}
	if percent == nil && limit != nil && used != nil {
		percent = provider.UsedPercentFromRatio(*used, *limit)
	}
	reading := provider.ProbeReading{AccountLabel: secret.Email, Windows: []core.QuotaWindow{}}
	if reading.AccountLabel == nil {
		reading.AccountLabel = reported(user.Email)
	}
	if tier := nonempty(reported(user.SubscriptionTier)); tier != nil {
		reading.Plan = &core.QuotaPlan{ID: *tier, Label: *tier}
	}
	if percent != nil || used != nil || limit != nil {
		id, label := "monthly", "Monthly credits"
		if kind := reported(period.Type); kind != nil && *kind == "USAGE_PERIOD_TYPE_WEEKLY" {
			id, label = "weekly", "Weekly credits"
		}
		reading.Windows = append(reading.Windows, core.QuotaWindow{ID: id, Label: label, UsedPercent: percent, Used: used, Limit: limit, Unit: new(core.QuotaUnitCredits), ResetsAt: reset, Severity: provider.Severity(percent)})
	}
	if cap := amountValue(config.OnDemandCap); cap != nil && *cap > 0 {
		reading.Windows = append(reading.Windows, core.QuotaWindow{ID: "on_demand_cap", Label: "On-demand cap", Limit: cap, Unit: new(core.QuotaUnitCredits), ResetsAt: reset})
	}
	return reading, nil
}
func (*quotaSource) Observe(o provider.Observation) ([]core.QuotaWindow, bool) {
	windows := []core.QuotaWindow{}
	for _, spec := range []struct {
		id, label, suffix string
		unit              core.QuotaUnit
	}{{"rpm", "Requests (short window)", "requests", core.QuotaUnitRequests}, {"tpm", "Tokens (short window)", "tokens", core.QuotaUnitTokens}} {
		limit := provider.HeaderNumber(o.Headers, "x-ratelimit-limit-"+spec.suffix)
		if limit == nil {
			continue
		}
		var used, percent *float64
		if remaining := provider.HeaderNumber(o.Headers, "x-ratelimit-remaining-"+spec.suffix); remaining != nil {
			used = new(*limit - *remaining)
			percent = provider.UsedPercentFromRatio(*used, *limit)
		}
		windows = append(windows, core.QuotaWindow{ID: spec.id, Label: spec.label, Limit: limit, Used: used, UsedPercent: percent, Unit: &spec.unit, Severity: provider.Severity(percent)})
	}
	return windows, len(windows) > 0
}
