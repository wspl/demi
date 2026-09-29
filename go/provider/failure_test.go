package provider_test

import (
	"errors"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

func TestFailureRecordsAndRetryWait(t *testing.T) {
	now, err := core.ParseTimestamp("2026-09-18T14:00:00.000Z")
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		value string
		ms    int64
	}{{"1", 1000}, {" 1.2349 ", 1234}, {"0.0009", 0}, {"0", 0}, {"Fri, 18 Sep 2026 14:00:02 GMT", 2000}} {
		got := provider.RetryAt(tc.value, now)
		if got == nil || got.Millisecond()-now.Millisecond() != tc.ms {
			t.Fatalf("%q: %v", tc.value, got)
		}
	}
	for _, value := range []string{"", "-5", "1e3", "0x10", "1.", ".5", "1.2.3", "2026-09-18T14:00:02Z", "9223372036854775807", "999999999999999999999999", "999999999999999"} {
		if got := provider.RetryAt(value, now); got != nil {
			t.Fatalf("accepted %q: %v", value, got)
		}
	}
	failure := provider.Refused("Vendor", 429, http.Header{"Retry-After": {"1.5"}}, "quota", provider.ReadHTTPFailure, now)
	if failure.Code != provider.RateLimit || failure.RetryAfter == nil || *failure.RetryAfter != 1500 {
		t.Fatal(failure)
	}
	record := provider.ReadHTTPFailureRecord(*failure.Diagnostics)
	if record == nil || record.Body != "quota" || record.Status != 429 {
		t.Fatal(record)
	}
	failure.Diagnostics.Source = core.FailureSourceStream
	if provider.ReadHTTPFailureRecord(*failure.Diagnostics) != nil {
		t.Fatal("read stream record as HTTP")
	}
	protocol := provider.ProtocolFailure("bad frame", "received")
	if protocol.Code != "" || protocol.Diagnostics.Source != core.FailureSourceStream || *protocol.Diagnostics.Upstream != "received" {
		t.Fatal(protocol)
	}
	transport := provider.TransportFailure("Vendor", &url.Error{Op: "Post", URL: "https://secret.invalid/token", Err: errors.New("connection reset")})
	if transport.Code != provider.Overloaded || strings.Contains(transport.Message, "secret.invalid") || transport.Diagnostics.Upstream != nil {
		t.Fatal(transport)
	}
	malformed := provider.EventStreamFailure("Vendor", provider.ErrSSEUTF8)
	if malformed.Code != "" || malformed.Diagnostics.Source != core.FailureSourceStream || malformed.Diagnostics.Upstream != nil {
		t.Fatal(malformed)
	}
	past := now
	failure = protocol.WithRetryWait(func(core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts {
		return core.ProviderFailureFacts{RetryAt: &past}
	}, now)
	if failure.RetryAfter == nil || *failure.RetryAfter != 0 {
		t.Fatal(failure)
	}
}
