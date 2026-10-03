package claudecode

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

type quotaSource struct {
	owner *Provider
	http  *http.Client
}

// ProbeCost reports the cost of a quota probe.
func (*quotaSource) ProbeCost() *provider.ProbeCost {
	cost := provider.ProbeFree
	return &cost
}

// Probe fetches the account quota windows.
func (q *quotaSource) Probe(ctx context.Context) (provider.ProbeReading, error) {
	secret, authErr := q.owner.stored(ctx)
	if authErr != nil {
		return provider.ProbeReading{}, authErr.QuotaError()
	}
	failed := func(err error) error {
		var u *url.Error
		if errors.As(err, &u) {
			err = u.Err
		}
		return &provider.QuotaError{
			Kind:    provider.QuotaUnavailable,
			Message: "Claude usage request failed: " + err.Error(),
		}
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, q.owner.config.UsageURL, nil)
	if err != nil {
		return provider.ProbeReading{}, failed(err)
	}
	request.Header.Set("Authorization", secret.AccessToken.Bearer().Expose())
	request.Header.Set("anthropic-beta", "oauth-2025-04-20")
	request.Header.Set("Accept", "application/json")
	response, err := q.http.Do(request)
	if err != nil {
		return provider.ProbeReading{}, failed(err)
	}
	defer func() {
		// The body is read to EOF; closing only releases the HTTP resource.
		_ = response.Body.Close()
	}()
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return provider.ProbeReading{}, failed(err)
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		chars := []rune(string(data))
		return provider.ProbeReading{}, &provider.QuotaError{
			Kind: provider.QuotaUnavailable,
			Message: fmt.Sprintf(
				"Claude usage request failed (%d): %s",
				response.StatusCode,
				string(chars[:min(200, len(chars))]),
			),
		}
	}
	return usageReading(data)
}

type quotaWindow struct {
	Utilization *float64                `json:"utilization"`
	ResetsAt    provider.ReportedString `json:"resets_at"   wire:"optional"`
}
type quotaLimit struct {
	Kind     provider.NonEmpty                     `json:"kind"`
	Percent  *float64                              `json:"percent"`
	Severity provider.Reported[core.QuotaSeverity] `json:"severity"  wire:"optional"`
	ResetsAt provider.ReportedString               `json:"resets_at" wire:"optional"`
	Scope    provider.Reported[limitScope]         `json:"scope"     wire:"optional"`
}
type limitScope struct {
	Model *struct {
		DisplayName *string `json:"display_name"`
	} `json:"model"`
}
type usageAnswer struct {
	FiveHour provider.Reported[quotaWindow]                     `json:"five_hour"        wire:"optional"`
	SevenDay provider.Reported[quotaWindow]                     `json:"seven_day"        wire:"optional"`
	Sonnet   provider.Reported[quotaWindow]                     `json:"seven_day_sonnet" wire:"optional"`
	Opus     provider.Reported[quotaWindow]                     `json:"seven_day_opus"   wire:"optional"`
	Limits   provider.Reported[[]provider.Reported[quotaLimit]] `json:"limits"           wire:"optional"`
}

var windowNames = []struct{ id, label string }{
	{"five_hour", "5h session"},
	{"seven_day", "7d all models"},
	{"seven_day_sonnet", "7d Sonnet"},
	{"seven_day_opus", "7d Opus"},
}

func namedWindow(id string, used *float64, reset *core.Timestamp) *core.QuotaWindow {
	for _, name := range windowNames {
		if name.id == id {
			unit := core.QuotaUnit("percent")
			return &core.QuotaWindow{
				ID:          id,
				Label:       name.label,
				UsedPercent: used,
				ResetsAt:    reset,
				Unit:        &unit,
				Severity:    provider.Severity(used),
			}
		}
	}
	return nil
}

func resetTime(text provider.ReportedString) *core.Timestamp {
	if text.Value == nil {
		return nil
	}
	return provider.RFC3339(*text.Value)
}

func percentage(value *float64) *float64 {
	if value == nil {
		return nil
	}
	return provider.ClampUsedPercent(*value)
}

func (u usageAnswer) windows() []core.QuotaWindow {
	windows := make([]core.QuotaWindow, 0)
	for i, w := range []provider.Reported[quotaWindow]{u.FiveHour, u.SevenDay, u.Sonnet, u.Opus} {
		if w.Value != nil {
			windows = append(
				windows,
				*namedWindow(windowNames[i].id, percentage(w.Value.Utilization), resetTime(w.Value.ResetsAt)),
			)
		}
	}
	if u.Limits.Value == nil {
		return windows
	}
	for _, reported := range *u.Limits.Value {
		l := reported.Value
		if l == nil || l.Kind == "session" || l.Kind == "weekly_all" {
			continue
		}
		kind := string(l.Kind)
		id, label := "limit:"+kind, kind
		scope := core.QuotaScope{Kind: kind}
		if l.Scope.Value != nil && l.Scope.Value.Model != nil && l.Scope.Value.Model.DisplayName != nil {
			model := *l.Scope.Value.Model.DisplayName
			id += ":" + model
			label += " (" + model + ")"
			scope = core.QuotaScope{Kind: "model", Label: &model}
		}
		used := percentage(l.Percent)
		severity := l.Severity.Value
		if severity == nil {
			severity = provider.Severity(used)
		}
		unit := core.QuotaUnit("percent")
		windows = append(
			windows,
			core.QuotaWindow{
				ID:          id,
				Label:       label,
				Scope:       &scope,
				UsedPercent: used,
				Severity:    severity,
				Unit:        &unit,
				ResetsAt:    resetTime(l.ResetsAt),
			},
		)
	}
	return windows
}

type unifiedWindow struct {
	Utilization provider.Reported[float64] `json:"utilization" wire:"optional"`
	ResetsAt    provider.Reported[int64]   `json:"resetsAt"    wire:"optional"`
}
type rateInfo struct {
	Type        provider.ReportedString                                        `json:"rateLimitType"  wire:"optional"`
	Utilization provider.Reported[float64]                                     `json:"utilization"    wire:"optional"`
	ResetsAt    provider.Reported[int64]                                       `json:"resetsAt"       wire:"optional"`
	Windows     provider.Reported[map[string]provider.Reported[unifiedWindow]] `json:"unifiedWindows" wire:"optional"`
}

// Observe reads quota windows from a provider observation.
func (*quotaSource) Observe(observation provider.Observation) []core.QuotaWindow {
	var raw json.RawMessage
	switch o := observation.(type) {
	case *provider.CLIObservation:
		raw = o.Line
	case *provider.HTTPObservation:
		return nil
	}
	line, err := provider.DecodeUntagged[struct {
		Type string    `json:"type"`
		Info *rateInfo `json:"rate_limit_info"`
	}](string(raw))
	if err != nil || line.Type != "rate_limit_event" || line.Info == nil {
		return nil
	}
	info := line.Info
	windows := make([]core.QuotaWindow, 0)
	add := func(id string, u unifiedWindow) {
		var used *float64
		var reset *core.Timestamp
		if u.Utilization.Value != nil {
			used = provider.ClampUsedPercent(*u.Utilization.Value * 100)
		}
		if u.ResetsAt.Value != nil {
			seconds := *u.ResetsAt.Value
			if seconds >= math.MinInt64/1000 && seconds <= math.MaxInt64/1000 {
				if timestamp, err := core.TimestampFromMillisecond(seconds * 1000); err == nil {
					reset = &timestamp
				}
			}
		}
		if w := namedWindow(id, used, reset); w != nil {
			windows = append(windows, *w)
		}
	}
	if info.Windows.Value != nil {
		ids := make([]string, 0, len(*info.Windows.Value))
		for id := range *info.Windows.Value {
			ids = append(ids, id)
		}
		slices.Sort(ids)
		for _, id := range ids {
			if w := (*info.Windows.Value)[id].Value; w != nil {
				add(id, *w)
			}
		}
	}
	if info.Type.Value != nil &&
		!slices.ContainsFunc(windows, func(w core.QuotaWindow) bool { return w.ID == *info.Type.Value }) {
		add(*info.Type.Value, unifiedWindow{Utilization: info.Utilization, ResetsAt: info.ResetsAt})
	}
	if len(windows) == 0 {
		return nil
	}
	return windows
}

func usageReading(data []byte) (provider.ProbeReading, error) {
	value, err := provider.DecodeUntagged[any](string(data))
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{
			Kind: provider.QuotaInvalid,
			Message: "Claude usage answer cannot be read: " +
				err.Error(),
		}
	}
	if _, ok := value.(map[string]any); !ok {
		return provider.ProbeReading{}, &provider.QuotaError{
			Kind:    provider.QuotaInvalid,
			Message: "Claude usage answer cannot be read: it is not an object",
		}
	}
	usage, err := provider.DecodeUntagged[usageAnswer](string(data))
	if err != nil {
		return provider.ProbeReading{}, &provider.QuotaError{
			Kind: provider.QuotaInvalid,
			Message: "Claude usage answer cannot be read: " +
				err.Error(),
		}
	}
	return provider.ProbeReading{Windows: usage.windows()}, nil
}
