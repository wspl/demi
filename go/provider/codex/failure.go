package codex

import (
	"encoding/json/jsontext"
	"encoding/json/v2"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

// ReadFailure reads reset fields from the vendor's preserved record, before Retry-After.
func ReadFailure(d core.ProviderErrorDiagnostics, received core.Timestamp) core.ProviderFailureFacts {
	var text *string
	if d.Source == core.FailureSourceHTTP {
		if record := provider.ReadHTTPFailureRecord(d); record != nil {
			text = &record.Body
		}
	} else {
		text = d.Upstream
	}
	if text != nil {
		var object map[string]jsontext.Value
		if json.Unmarshal([]byte(*text), &object) == nil {
			var limit map[string]jsontext.Value
			for _, key := range []string{"error", "event", "response"} {
				raw := object[key]
				if key != "error" {
					var nested map[string]jsontext.Value
					if json.Unmarshal(raw, &nested) != nil {
						continue
					}
					raw = nested["error"]
				}
				if raw.Kind() == '{' && json.Unmarshal(raw, &limit) == nil {
					break
				}
			}
			for _, key := range []string{"resets_at", "resets_in_seconds"} {
				raw := limit[key]
				if raw.Kind() != '0' {
					continue
				}
				var seconds float64
				if json.Unmarshal(raw, &seconds) != nil {
					continue
				}
				if key == "resets_in_seconds" {
					seconds += float64(received.Millisecond()) / 1000
				}
				return core.ProviderFailureFacts{RetryAt: provider.UnixSeconds(seconds)}
			}
		}
	}
	return provider.ReadHTTPFailure(d, received)
}
