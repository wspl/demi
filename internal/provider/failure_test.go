package provider_test

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestHTTPStatusCodes(t *testing.T) {
	for _, tc := range []struct {
		status  int
		message string
		code    provider.ErrorCode
	}{
		{401, "", provider.AuthExpired},
		{403, "", provider.AuthExpired},
		{429, "", provider.RateLimit},
		{408, "", provider.Overloaded},
		{409, "", provider.Overloaded},
		{425, "", provider.Overloaded},
		{500, "", provider.Overloaded},
		{529, "", provider.Overloaded},
		{400, "prompt is too long: 213000 tokens", provider.ContextLengthExceeded},
		{400, "Context length exceeded", provider.ContextLengthExceeded},
		{413, "", provider.ContextLengthExceeded},
		{400, "Request too large", provider.ContextLengthExceeded},
		{400, "Too many images in request", provider.ContextLengthExceeded},
		{
			400,
			`image dimensions exceed max allowed size for many-image ` +
				`requests: 2000 pixels`,
			provider.ContextLengthExceeded,
		},
		{400, "bad request", ""},
		{404, "", ""},
	} {
		code := provider.HTTPErrorCode(tc.status, tc.message)
		if tc.code == "" {
			if code != nil {
				t.Fatalf("unexpected code %s", *code)
			}
		} else if code == nil || *code != tc.code {
			t.Fatalf("%d %s: %v", tc.status, tc.message, code)
		}
	}
}

func TestVendorClassification(t *testing.T) {
	for _, tc := range []struct {
		code, message string
		want          provider.ErrorCode
	}{
		{"", "maximum context length", provider.ContextLengthExceeded},
		{"", "max_tokens: 64000 > 32000 tokens", provider.ContextLengthExceeded},
		{"request_too_large", "Request exceeds the maximum size", provider.ContextLengthExceeded},
		{"invalid_request_error", "Too many images in request", provider.ContextLengthExceeded},
		{
			"invalid_request_error",
			"image dimensions exceed max allowed size for many-image requests",
			provider.ContextLengthExceeded,
		},
		{"", "rate limit reached", provider.RateLimit},
		{"rate_limit_error", "Number of requests exceeded", provider.RateLimit},
		{"", "insufficient balance", provider.RateLimit},
		{"insufficient_quota", "You exceeded your plan", provider.RateLimit},
		{"usage_limit_reached", "The usage limit has been reached", provider.RateLimit},
		{"", "invalid api key", provider.AuthExpired},
		{"authentication_error", "invalid x-api-key", provider.AuthExpired},
		{"", "service unavailable", provider.Overloaded},
		{"overloaded_error", "Overloaded", provider.Overloaded},
		{"server_error", "backend failed", provider.Overloaded},
		{"internal-error", "backend failed", provider.Overloaded},
		{"api_error", "Internal server error", provider.Overloaded},
		{"", "Codex SSE response headers timed out after 20000ms", provider.Overloaded},
		{"", "connect timeout", provider.Overloaded},
		{"", "socket hang up", provider.Overloaded},
		{"", "read ECONNRESET", provider.Overloaded},
		{"invalid_request_error", "Invalid prompt_cache_key", "invalid_request_error"},
		{"custom", "something else", "custom"},
		{"invalid_request_error", "Could not generate the schema", "invalid_request_error"},
		{"invalid_request_error", "tools: exceeds the limit of 128", "invalid_request_error"},
		{"", "iterate over the unlimited list", ""},
		{"invalid_request_error", "Invalid usage of the tools parameter", "invalid_request_error"},
		{"", "usage limit exceeded for this month", provider.RateLimit},
		{"", "nothing to see", ""},
	} {
		var vendorCode *string
		if tc.code != "" || tc.message == "nothing to see" {
			vendorCode = &tc.code
		}
		code := provider.ClassifyError(vendorCode, tc.message)
		if tc.want == "" {
			if code != nil {
				t.Fatalf("%s: %s", tc.message, *code)
			}
		} else if code == nil || *code != tc.want {
			t.Fatalf("%s %s: %v", tc.code, tc.message, code)
		}
	}
}

func TestHTTPRecordHeaderOrder(t *testing.T) {
	headers := http.Header{}
	headers.Add("X-Request-Id", "req-9")
	headers.Add("Set-Cookie", "b=2")
	headers.Add("retry-after", "120")
	headers.Add("set-cookie", "a=1")
	record := provider.NewHTTPFailureRecord(429, headers, `{"error":{"message":"slow down"}}`)
	requireEqual(
		t,
		encoded(t, record),
		`{"status":429,"headers":[["retry-after","120"],["set-cookie",`+
			`"b=2"],["set-cookie","a=1"],["x-request-id","req-9"]],`+
			`"body":"{\"error\":{\"message\":\"slow down\"}}"}`,
	)
}

func TestHTTPFailureRecordAndWait(t *testing.T) {
	vendor := providertest.StartVendor(t)
	vendor.Respond(
		providertest.MockResponse{
			Status: 429,
			Headers: http.Header{
				"Retry-After":  []string{"120"},
				"X-Request-Id": []string{"req-9"},
				"Set-Cookie":   []string{"a=b"},
			},
			Chunks: [][]byte{[]byte(`{"error":{"message":"slow down"}}`)},
		},
	)
	vendor.Respond(providertest.MockResponse{Status: 502, Chunks: [][]byte{[]byte("<html>Bad gateway</html>")}})
	for _, tc := range []struct {
		status int
		body   string
		code   provider.ErrorCode
		wait   time.Duration
	}{
		{
			429,
			`{"error":{"message":"slow down"}}`,
			provider.RateLimit,
			120 * time.Second,
		}, {
			502,
			"<html>Bad gateway</html>",
			provider.Overloaded,
			0,
		},
	} {
		request, err := http.NewRequestWithContext(t.Context(), http.MethodGet, vendor.URL("/v1/messages"), nil)
		if err != nil {
			t.Fatal(err)
		}
		response, err := vendor.Client().Do(request)
		if err != nil {
			t.Fatal(err)
		}
		failure := provider.HTTPFailure(
			t.Context(),
			response,
			"Acme",
			provider.ReadHTTPFailure,
			providertest.FixedClock(now),
		)
		if failure.Code == nil || *failure.Code != tc.code ||
			failure.Message != fmt.Sprintf("Acme API request failed with HTTP %d: %s", tc.status, tc.body) {
			t.Fatalf("%+v", failure)
		}
		if tc.wait != 0 {
			if failure.RetryAfter == nil || *failure.RetryAfter != tc.wait {
				t.Fatalf("%+v", failure)
			}
		} else if failure.RetryAfter != nil {
			t.Fatal("unexpected wait")
		}
		record, ok := provider.ReadHTTPRecord(failure.Diagnostics)
		if !ok || int(record.Status) != tc.status || record.Body != tc.body ||
			failure.Diagnostics.Source != "http" {
			t.Fatalf("%+v", record)
		}
		requireEqual(t, *failure.Diagnostics.HTTPStatus, uint16(tc.status))
		if tc.status == 429 {
			fields, err := contract.ObjectFields([]byte(*failure.Diagnostics.Upstream))
			if err != nil {
				t.Fatal(err)
			}
			names := make([]string, len(fields))
			for i, field := range fields {
				names[i] = field.Name
			}
			requireEqual(t, names, []string{"status", "headers", "body"})
			names = names[:0]
			for _, pair := range record.Headers {
				names = append(names, pair[0])
			}
			if !slices.IsSorted(names) {
				t.Fatalf("unsorted headers: %v", names)
			}
			retryAfter, _ := record.Header("retry-after")
			requireEqual(t, retryAfter, "120")
			requestID, _ := record.Header("x-request-id")
			requireEqual(t, requestID, "req-9")
			cookie, _ := record.Header("set-cookie")
			requireEqual(t, cookie, "a=b")
		}
	}
}

func TestRetryAfter(t *testing.T) {
	for _, tc := range []struct{ value, want string }{
		{"120", "2026-09-18T14:02:00.000Z"},
		{" 120\t", "2026-09-18T14:02:00.000Z"},
		{"0", string(now)},
		{"1.5", "2026-09-18T14:00:01.500Z"},
		{"1.2349", "2026-09-18T14:00:01.234Z"},
		{"Tue, 22 Sep 2026 07:37:39 GMT", "2026-09-22T07:37:39.000Z"},
		{"Tuesday, 22-Sep-26 07:37:39 GMT", "2026-09-22T07:37:39.000Z"},
		{"Tue Sep 22 07:37:39 2026", "2026-09-22T07:37:39.000Z"},
	} {
		got := provider.RetryAt(tc.value, now)
		if got == nil || string(*got) != tc.want {
			t.Fatalf("%q: %v", tc.value, got)
		}
	}
	for _, text := range []string{
		"1e3",
		"0x10",
		"2026-09-22T07:37:39Z",
		"-5",
		"+5",
		"soon",
		"",
		"1.",
		".5",
		"99999999999999999999",
	} {
		if got := provider.RetryAt(text, now); got != nil {
			t.Fatalf("accepted %s: %v", text, *got)
		}
	}
}

func TestStandardHTTPReading(t *testing.T) {
	record := provider.HTTPFailureRecord{
		Status:  429,
		Headers: []provider.HeaderPair{{"retry-after", "90"}},
		Body:    "slow down",
	}
	upstream := encoded(t, record)
	diagnostics := core.ProviderErrorDiagnostics{Source: "http", Upstream: &upstream}
	requireEqual(t, *provider.ReadHTTPFailure(&diagnostics, now).RetryAt, core.Timestamp("2026-09-18T14:01:30.000Z"))
	for _, tc := range []struct {
		source core.FailureSource
		record string
	}{{
		"http",
		`{"status":429,"headers":[],"body":"slow down"}`,
	}, {
		"stream",
		`{"retry-after":"90"}`,
	}, {
		"http",
		"not a record",
	}} {
		diagnostics := core.ProviderErrorDiagnostics{Source: tc.source, Upstream: &tc.record}
		if provider.ReadHTTPFailure(&diagnostics, now).RetryAt != nil {
			t.Fatalf("unexpected time: %+v", tc)
		}
	}
}

func TestRetryWaitOnlyWhenNamed(t *testing.T) {
	failure := provider.ProtocolFailure("x", "x")
	for _, tc := range []struct {
		at   core.Timestamp
		wait time.Duration
	}{{"2026-09-18T14:00:30.000Z", 30 * time.Second}, {"2026-09-18T13:00:00.000Z", 0}} {
		got := failure.WithRetryWait(func(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts {
			return core.ProviderFailureFacts{RetryAt: &tc.at}
		}, now)
		if got.RetryAfter == nil || *got.RetryAfter != tc.wait {
			t.Fatalf("%+v", got)
		}
	}
	none := func(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts {
		return core.ProviderFailureFacts{}
	}
	requireEqual(t, failure.WithRetryWait(none, now), failure)
	failure.Diagnostics = nil
	later := func(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts {
		at := core.Timestamp("2026-09-18T14:00:30.000Z")
		return core.ProviderFailureFacts{RetryAt: &at}
	}
	requireEqual(t, failure.WithRetryWait(later, now), failure)
}

func TestTransportFailureOmitsEndpoint(t *testing.T) {
	// A controlled dial failure does not race with reuse of a temporarily free port.
	transport := &http.Transport{
		DialContext: func(context.Context, string, string) (net.Conn, error) { return nil, errors.New("connection refused") },
	}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport}
	request, err := http.NewRequestWithContext(t.Context(), http.MethodGet, "http://127.0.0.1:1/v1/messages", nil)
	if err != nil {
		t.Fatal(err)
	}
	_, err = client.Do(request)
	failure := provider.TransportFailure("Acme", err)
	if failure.Code == nil || *failure.Code != provider.Overloaded ||
		!strings.HasPrefix(failure.Message, "Acme API request failed: ") ||
		strings.Contains(failure.Message, "127.0.0.1") {
		t.Fatalf("%+v", failure)
	}
	requireEqual(t, failure.Diagnostics.Source, core.FailureSource("transport"))
	if failure.Diagnostics.Upstream != nil {
		t.Fatal("transport has a record")
	}
}

func TestRequestBuildFailureHasNoRetryCode(t *testing.T) {
	_, err := http.NewRequestWithContext(t.Context(), "invalid method", "https://secret.invalid/path", nil)
	if err == nil {
		t.Fatal("constructed invalid request")
	}
	failure := provider.RequestBuildFailure("Acme", err)
	if failure.Code != nil || failure.Diagnostics.Source != "transport" || failure.Diagnostics.Upstream != nil ||
		strings.Contains(failure.Message, "secret.invalid") {
		t.Fatalf("%#v", failure)
	}
}

func TestHTTPRecordReplacesEachMalformedSequenceOnce(t *testing.T) {
	response := &http.Response{
		StatusCode: 400,
		Header:     http.Header{"X-Vendor": []string{"a\xff\xff\xe2\x82"}},
		Body:       io.NopCloser(strings.NewReader("a\xff\xff\xe2\x82")),
	}
	failure := provider.HTTPFailure(
		t.Context(),
		response,
		"Acme",
		provider.ReadHTTPFailure,
		providertest.FixedClock(now),
	)
	record, _ := provider.ReadHTTPRecord(failure.Diagnostics)
	requireEqual(t, record.Body, "a\ufffd\ufffd\ufffd")
	vendor, _ := record.Header("x-vendor")
	requireEqual(t, vendor, record.Body)
}
