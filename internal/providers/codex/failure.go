package codex

import (
	"encoding/json"
	"net/http"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

func readFailure(d *core.ProviderErrorDiagnostics, now core.Timestamp) core.ProviderFailureFacts {
	if d == nil {
		return core.ProviderFailureFacts{}
	}
	text := d.Upstream
	if d.Source == core.FailureSourceHTTP {
		text = nil
		if record, ok := provider.ReadHTTPRecord(d); ok {
			text = &record.Body
		}
	}
	if text == nil {
		return provider.ReadHTTPFailure(d, now)
	}
	obj, err := provider.DecodeUntagged[map[string]json.RawMessage](*text)
	if err != nil {
		return provider.ReadHTTPFailure(d, now)
	}
	raw := obj["error"]
	limit, e := provider.DecodeUntagged[map[string]json.RawMessage](string(raw))
	for _, key := range []string{"event", "response"} {
		if e == nil && limit != nil {
			break
		}
		outer, _ := provider.DecodeUntagged[map[string]json.RawMessage](string(obj[key]))
		limit, e = provider.DecodeUntagged[map[string]json.RawMessage](string(outer["error"]))
	}
	if value, e := provider.DecodeUntagged[float64](string(limit["resets_at"])); e == nil {
		return core.ProviderFailureFacts{RetryAt: provider.UnixSeconds(value)}
	}
	if value, e := provider.DecodeUntagged[float64](string(limit["resets_in_seconds"])); e == nil {
		ms, _ := now.Millisecond()
		return core.ProviderFailureFacts{RetryAt: provider.UnixSeconds(float64(ms)/1000 + value)}
	}
	return provider.ReadHTTPFailure(d, now)
}

type refusalError struct {
	status  int
	headers http.Header
	body    string
}

// Error returns the HTTP status text of the refusal.
func (r *refusalError) Error() string { return http.StatusText(r.status) }

func (p *Provider) refused(r *refusalError) provider.Failure {
	body, err := provider.DecodeUntagged[errorBody](r.body)
	if err != nil {
		body = errorBody{}
	} // A malformed refusal leaves the status to speak.

	f := provider.Refused("Codex", uint16(r.status), r.headers, r.body, readFailure, p.clock.Now())
	if f.Diagnostics != nil {
		if body.Error.Value != nil {
			f.Diagnostics.ProviderCode = body.Error.Value.Code.Value
			if f.Diagnostics.ProviderCode == nil {
				f.Diagnostics.ProviderCode = body.Error.Value.Kind.Value
			}
		}
		f.Diagnostics.ProviderRequestID = body.RequestID.Value
		if id := r.headers.Get("X-Request-Id"); id != "" {
			f.Diagnostics.ProviderRequestID = &id
		}
	}
	return f
}

// errorBody declares only the fields Codex reports about an HTTP refusal.
type errorBody struct {
	Error     provider.Reported[errorFields] `json:"error"      wire:"optional"`
	RequestID provider.ReportedString        `json:"request_id" wire:"optional"`
}
type errorFields struct {
	Code provider.ReportedString `json:"code" wire:"optional"`
	Kind provider.ReportedString `json:"type" wire:"optional"`
}
