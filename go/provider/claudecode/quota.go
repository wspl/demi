package claudecode

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"io"
	"net/http"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire open
type quotaWindow struct {
	Utilization    *float64                 `json:"utilization,omitzero" check:"nullabsent"`
	UsedPercentage *float64                 `json:"used_percentage,omitzero" check:"nullabsent"`
	ResetsAt       *provider.ReportedString `json:"resets_at,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:wire open
type quotaLimit struct {
	Kind     string                   `json:"kind" check:"chars=1.."`
	Percent  *float64                 `json:"percent,omitzero" check:"nullabsent"`
	Severity *provider.ReportedString `json:"severity,omitzero" check:"nullabsent,func=provider.Validate"`
	ResetsAt *provider.ReportedString `json:"resets_at,omitzero" check:"nullabsent,func=provider.Validate"`
	Scope    *reportedScope           `json:"scope,omitzero" check:"nullabsent"`
}

//demi:wire open
type limitScope struct {
	Model *scopedModel `json:"model,omitzero" check:"nullabsent"`
}

//demi:wire open
type scopedModel struct {
	DisplayName *string `json:"display_name,omitzero" check:"nullabsent"`
}

//demi:opaque
type reportedScope struct{ value *limitScope }

func (v *reportedScope) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.value = nil
	if value, err := decode[limitScope](raw); err == nil {
		v.value = &value
	}
	return nil
}

type quotaSource struct{ p *Provider }

func (*quotaSource) ProbeCost() *provider.ProbeCost { return new(provider.ProbeFree) }
func (q *quotaSource) Probe(ctx context.Context) (provider.ProbeReading, error) {
	secret, failure := q.p.stored(ctx)
	if failure != nil {
		return provider.ProbeReading{}, failure.QuotaError()
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, q.p.config.UsageURL.String(), nil)
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: "Claude usage request could not be built"}
	}
	request.Header = http.Header{"Authorization": {secret.AccessToken.Bearer()}, "Anthropic-Beta": {"oauth-2025-04-20"}, "Accept": {"application/json"}}
	response, err := q.p.http.Do(request)
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: provider.TransportFailure("Claude usage", err).Message}
	}
	defer response.Body.Close()
	raw, err := io.ReadAll(response.Body)
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: "Claude usage response could not be read"}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		text := []rune(string(raw))
		if len(text) > 200 {
			text = text[:200]
		}
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaUnavailable, Message: fmt.Sprintf("Claude usage request failed (%d): %s", response.StatusCode, string(text))}
	}
	windows, ok := usage(raw)
	if !ok {
		return provider.ProbeReading{}, &provider.QuotaError{Kind: provider.QuotaInvalid, Message: "Claude usage answer cannot be read: expected an object"}
	}
	return provider.ProbeReading{Windows: windows}, nil
}
func (*quotaSource) Observe(o provider.Observation) ([]core.QuotaWindow, bool) {
	var object map[string]jsontext.Value
	if json.Unmarshal(o.CLILine, &object) != nil {
		return nil, false
	}
	raw, ok := object["rate_limits"]
	if !ok {
		if json.Unmarshal(object["message"], &object) != nil {
			return nil, false
		}
		raw = object["rate_limits"]
	}
	windows, ok := usage(raw)
	return windows, ok && len(windows) > 0
}

// usage reads each reported window independently, so a new or malformed display
// field cannot hide the other windows the account reports.
func usage(raw jsontext.Value) ([]core.QuotaWindow, bool) {
	if raw.Kind() != '{' {
		return nil, false
	}
	var object map[string]jsontext.Value
	if json.Unmarshal(raw, &object) != nil {
		return nil, false
	}
	windows := []core.QuotaWindow{}
	for _, named := range []struct{ id, label string }{{"five_hour", "5h session"}, {"seven_day", "7d all models"}, {"seven_day_sonnet", "7d Sonnet"}, {"seven_day_opus", "7d Opus"}} {
		window, err := decode[quotaWindow](object[named.id])
		if err != nil {
			continue
		}
		percent := window.Utilization
		if percent == nil {
			percent = window.UsedPercentage
		}
		if percent != nil {
			percent = provider.ClampUsedPercent(*percent)
		}
		windows = append(windows, core.QuotaWindow{ID: named.id, Label: named.label, UsedPercent: percent, Unit: new(core.QuotaUnitPercent), ResetsAt: reset(window.ResetsAt), Severity: provider.Severity(percent)})
	}
	var limits []jsontext.Value
	if json.Unmarshal(object["limits"], &limits) == nil {
		for _, raw := range limits {
			limit, err := decode[quotaLimit](raw)
			if err != nil || limit.Kind == "session" || limit.Kind == "weekly_all" {
				continue
			}
			percent := limit.Percent
			if percent != nil {
				percent = provider.ClampUsedPercent(*percent)
			}
			id, label := "limit:"+limit.Kind, limit.Kind
			scope := core.QuotaScope{Kind: limit.Kind}
			if limit.Scope != nil && limit.Scope.value != nil && limit.Scope.value.Model != nil && limit.Scope.value.Model.DisplayName != nil {
				model := *limit.Scope.value.Model.DisplayName
				id += ":" + model
				label += " (" + model + ")"
				scope = core.QuotaScope{Kind: "model", Label: &model}
			}
			severity := provider.Severity(percent)
			if limit.Severity != nil && limit.Severity.Value != nil {
				candidate := core.QuotaSeverity(*limit.Severity.Value)
				if core.Validate(candidate) == nil {
					severity = &candidate
				}
			}
			windows = append(windows, core.QuotaWindow{ID: id, Label: label, UsedPercent: percent, Unit: new(core.QuotaUnitPercent), ResetsAt: reset(limit.ResetsAt), Severity: severity, Scope: &scope})
		}
	}
	return windows, true
}
func reset(value *provider.ReportedString) *core.Timestamp {
	if value == nil || value.Value == nil {
		return nil
	}
	return provider.RFC3339(*value.Value)
}
