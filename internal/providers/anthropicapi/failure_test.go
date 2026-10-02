package anthropicapi_test

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestRefusedRequestRecordAndWait(t *testing.T) {
	body := `{"type":"error","error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit"}}`
	failure := onlyFailure(t, eventsOf(t, providertest.MockResponse{Status: 429, Headers: http.Header{"Content-Type": {"application/json"}, "Retry-After": {"30"}, "Request-Id": {"req_1"}}, Chunks: [][]byte{[]byte(body)}}))
	if failure.Message != "Anthropic API request failed with HTTP 429: "+body || failure.Code == nil || *failure.Code != provider.RateLimit || failure.RetryAfter == nil || *failure.RetryAfter != 30*time.Second {
		t.Fatalf("%+v", failure)
	}
	d := failure.Diagnostics
	if d.Source != "http" || *d.HTTPStatus != 429 {
		t.Fatal(d)
	}
	record := provider.ReadHTTPRecord(d)
	if record == nil || record.Body != body || record.Header("request-id") == nil || *record.Header("request-id") != "req_1" {
		t.Fatal(record)
	}
}
func TestRefusalStatusCode(t *testing.T) {
	for _, c := range []struct {
		status int
		body   string
		code   provider.ErrorCode
	}{
		{529, `{"error":{"type":"overloaded_error","message":"Overloaded"}}`, provider.Overloaded},
		{401, `{"error":{"type":"authentication_error","message":"invalid x-api-key"}}`, provider.AuthExpired},
		{400, `{"error":{"type":"invalid_request_error","message":"prompt is too long: 213000 tokens > 200000 maximum"}}`, provider.ContextLengthExceeded},
		{404, `{"error":{"type":"not_found_error","message":"model: claude-x"}}`, ""},
	} {
		failure := onlyFailure(t, eventsOf(t, providertest.MockResponse{Status: c.status, Chunks: [][]byte{[]byte(c.body)}}))
		if c.code == "" {
			if failure.Code != nil {
				t.Fatal(failure)
			}
		} else if failure.Code == nil || *failure.Code != c.code {
			t.Fatal(failure)
		}
		if failure.RetryAfter != nil {
			t.Fatal(failure)
		}
	}
}
func TestNoAnswer(t *testing.T) {
	v := providertest.StartVendor(t)
	r := testRuntime(t, v, provider.VendorPolicy{})
	endpoint := v.URL("/v1")
	v.Close()
	failure := onlyFailure(t, providertest.Run(t.Context(), t, r, providertest.InferenceRequest()))
	if failure.Code == nil || *failure.Code != provider.Overloaded || !strings.HasPrefix(failure.Message, "Anthropic API request failed: ") || strings.Contains(failure.Message, strings.TrimPrefix(endpoint, "http://")) || failure.Diagnostics.Source != "transport" {
		t.Fatalf("%+v", failure)
	}
}
