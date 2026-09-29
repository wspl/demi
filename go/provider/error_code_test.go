package provider_test

import (
	"testing"

	"github.com/wspl/demi/go/provider"
)

func TestHTTPErrorCodes(t *testing.T) {
	for _, tc := range []struct {
		status  uint16
		message string
		want    provider.ErrorCode
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
		{400, "image dimensions exceed max allowed size for many-image requests: 2000 pixels", provider.ContextLengthExceeded},
		{400, "bad request", ""},
		{404, "", ""},
	} {
		if got := provider.HTTPErrorCode(tc.status, tc.message); got != tc.want {
			t.Errorf("%d %s: %s want %s", tc.status, tc.message, got, tc.want)
		}
	}
}
func TestVendorErrorWordsAndPrecedence(t *testing.T) {
	for _, tc := range []struct {
		code, message string
		want          provider.ErrorCode
	}{
		{"", "maximum context length", provider.ContextLengthExceeded},
		{"", "max_tokens: 64000 > 32000 tokens", provider.ContextLengthExceeded},
		{"request_too_large", "Request exceeds the maximum size", provider.ContextLengthExceeded},
		{"invalid_request_error", "Too many images in request", provider.ContextLengthExceeded},
		{"invalid_request_error", "image dimensions exceed max allowed size for many-image requests", provider.ContextLengthExceeded},
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
		if got := provider.ClassifyVendorFailure(tc.code, tc.message); got != tc.want {
			t.Errorf("%s %s: %s want %s", tc.code, tc.message, got, tc.want)
		}
	}
}
