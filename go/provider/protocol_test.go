package provider_test

import (
	"encoding/base64"
	"encoding/json/v2"
	"fmt"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/internal/testfixture"
	"github.com/wspl/demi/go/provider/providertest"
)

func TestSecretRejectsControlAndNeverFormatsText(t *testing.T) {
	for _, bad := range []string{"", "sk-1\nx", "x\u0085y", "x\ty", "x\x00y"} {
		if _, err := provider.NewSecret(bad); err == nil {
			t.Fatal("accepted invalid secret")
		}
	}
	secret, err := provider.NewSecret("made-up-key")
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"%s", "%v", "%+v", "%#v", "%q"} {
		if got := fmt.Sprintf(format, secret); got != "Secret(..)" {
			t.Fatalf("unsafe format %s", format)
		}
	}
	var decoded provider.Secret
	if err := json.Unmarshal([]byte(`"made-up-key"`), &decoded); err != nil || decoded.Expose() != secret.Expose() {
		t.Fatal("secret decode failed", err)
	}
	if err := json.Unmarshal([]byte(`""`), &decoded); err == nil {
		t.Fatal("accepted empty JSON credential")
	}
}
func TestHTTPRecordPreservesHeadersAndBody(t *testing.T) {
	headers := make(http.Header)
	headers.Add("X-Request-Id", "req-9")
	headers.Add("Set-Cookie", "b=2")
	headers.Add("retry-after", "120")
	headers.Add("set-cookie", "a=1")
	record := provider.NewHTTPFailureRecord(429, headers, `{"error":{"message":"slow down"}}`)
	text, err := record.JSON()
	want := `{"status":429,"headers":[["retry-after","120"],["set-cookie","b=2"],["set-cookie","a=1"],["x-request-id","req-9"]],"body":"{\"error\":{\"message\":\"slow down\"}}"}`
	if err != nil || string(text) != want {
		t.Fatalf("record=%s error=%v", text, err)
	}
	var read provider.HTTPFailureRecord
	if err := json.Unmarshal(text, &read); err != nil || !reflect.DeepEqual(read, record) {
		t.Fatal("record could not be read", err)
	}
	if value, ok := read.Header("RETRY-AFTER"); !ok || value != "120" {
		t.Fatal("header reading failed")
	}
	for _, bad := range []string{`{}`, `{"status":429,"headers":[["a"]],"body":""}`, `{"status":429,"headers":[["a","b","c"]],"body":""}`, `{"status":429,"headers":[],"body":"","extra":1}`} {
		if err := json.Unmarshal([]byte(bad), &read); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
}
func TestOAuthSecondsAndFallbacks(t *testing.T) {
	for _, tc := range []struct {
		json    string
		seconds float64
	}{{"5", 5}, {`"28800"`, 28800}, {`" 1.5 "`, 1.5}, {"0", 0}} {
		var got provider.OAuthSeconds
		if err := json.Unmarshal([]byte(tc.json), &got); err != nil || got.Seconds != tc.seconds {
			t.Fatalf("%s: %+v %v", tc.json, got, err)
		}
	}
	for _, bad := range []string{`"1e3"`, `"soon"`, `-1`, `true`, `null`, `"-5"`} {
		var got provider.OAuthSeconds
		if err := json.Unmarshal([]byte(bad), &got); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	var interval provider.PollInterval
	if interval.Seconds() != 5 {
		t.Fatal("wrong absent interval")
	}
	for _, tc := range []struct {
		text string
		want float64
	}{{`"soon"`, 5}, {`"0"`, 0}, {`3`, 3}, {`null`, 5}} {
		if err := json.Unmarshal([]byte(tc.text), &interval); err != nil || interval.Seconds() != tc.want {
			t.Fatalf("interval %s: %v %v", tc.text, interval.Seconds(), err)
		}
	}
	for _, tc := range []struct {
		text    string
		present bool
	}{{`"600"`, true}, {`0`, false}, {`"forever"`, false}, {`-3`, false}} {
		var lifetime provider.Lifetime
		if err := json.Unmarshal([]byte(tc.text), &lifetime); err != nil || (lifetime.Seconds != nil) != tc.present {
			t.Fatalf("lifetime %s: %+v %v", tc.text, lifetime, err)
		}
	}
}
func TestJWTClaimsAndCacheIDs(t *testing.T) {
	for _, encoding := range []*base64.Encoding{base64.RawURLEncoding, base64.URLEncoding} {
		payload := encoding.EncodeToString([]byte(`{"access":"zoé@example.com","refresh":"r"}`))
		got, ok := provider.JWTClaims("h."+payload+".s", testfixture.DecodeTokens)
		if !ok || got.Access != "zoé@example.com" {
			t.Fatal("claims not decoded")
		}
	}
	for _, bad := range []string{"header.payload", "", "a..c", "header.!!!!.signature", "a.b.c.d", "header.bm9wZQ.sig"} {
		if _, ok := provider.JWTClaims(bad, testfixture.DecodeTokens); ok {
			t.Fatalf("accepted %s", bad)
		}
	}
	for _, tc := range []struct{ text, want string }{{strings.Repeat("x", 64), strings.Repeat("x", 64)}, {"chat-" + strings.Repeat("x", 80), "session_cc30bf2"}, {strings.Repeat("😀", 33), "session_21a3538"}, {strings.Repeat("会话", 40), "session_1bc4bf95"}} {
		if got := provider.PromptCacheKey(tc.text); got != tc.want {
			t.Fatalf("cache key=%s want=%s", got, tc.want)
		}
	}
}
func TestTaggedVendorPayloads(t *testing.T) {
	registry := map[string]func([]byte) (testfixture.Event, error){"start": testfixture.DecodeEvent}
	got, ok, err := provider.DecodeTagged([]byte(`{"type":"start","index":2,"text":"hi","vendor_field":1}`), registry)
	if err != nil || !ok || got.Text != "hi" || got.Index != 2 {
		t.Fatalf("%+v %v %v", got, ok, err)
	}
	if _, ok, err := provider.DecodeTagged([]byte(`{"type":"future","arbitrary":[]}`), registry); ok || err != nil {
		t.Fatal("unknown tag refused")
	}
	for _, bad := range []string{`{}`, `{"type":5}`, `not json`, ``, `{"type":"start","text":"hi"}`, `{"type":"start","index":0,"text":42}`} {
		if _, _, err := provider.DecodeTagged([]byte(bad), registry); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	for _, bad := range []string{`null`, `12`, `{}`, `[]`, `false`} {
		var reported provider.ReportedString
		if err := json.Unmarshal([]byte(bad), &reported); err != nil || reported.Value != nil {
			t.Fatalf("reported %s: %+v %v", bad, reported, err)
		}
	}
}
func TestSecretDecodeErrorsNamePathWithoutValues(t *testing.T) {
	for _, tc := range []struct{ text, path, fault string }{
		{`{"access":"sk-secret","refresh":7}`, "refresh", "a field is missing, unknown or of the wrong type"},
		{`{"access":"sk-secret"}`, ".", "a field is missing, unknown or of the wrong type"},
		{`{"access":"sk-secret","refresh":"r","extra":"sk-secret"}`, "extra", "a field is missing, unknown or of the wrong type"},
		{`{"access":"sk-secret"`, ".", "not JSON"},
		{`{"access":"sk-secret","refresh":"r"} trailing`, ".", "not JSON"},
	} {
		_, err := provider.DecodeSecret([]byte(tc.text), testfixture.DecodeTokens)
		want := fmt.Sprintf("malformed at %s: %s", tc.path, tc.fault)
		if err == nil || err.Error() != want {
			t.Fatalf("got %v want %s", err, want)
		}
	}
}

func TestOAuthResponseNeverQuotesTokens(t *testing.T) {
	vendor := providertest.NewMockVendor(t,
		providertest.MockResponse{Chunks: []string{`{"access":"made-up-token","refresh":"r"}`}},
		providertest.MockResponse{Chunks: []string{`{"access":"made-up-token","refresh":42}`}},
		providertest.MockResponse{Chunks: []string{`{"access":"made-up-token"`}, Ending: providertest.Broken},
	)
	for _, want := range []string{"", "the response is malformed at refresh: a field is missing, unknown or of the wrong type", "the response body could not be read"} {
		response, err := vendor.Server.Client().Get(vendor.Server.URL)
		if err != nil {
			t.Fatal(err)
		}
		got, err := provider.DecodeOAuthResponse(response, testfixture.DecodeTokens)
		if want == "" {
			if err != nil || got.Access != "made-up-token" {
				t.Fatalf("valid response: %+v %v", got, err)
			}
		} else if err == nil || err.Error() != want {
			t.Fatalf("got %v want %s", err, want)
		}
	}
}
func TestEndpointAppendsOnceAndKeepsBase(t *testing.T) {
	for _, tc := range []struct{ base, want string }{
		{"https://vendor.invalid/v1/", "https://vendor.invalid/v1/responses"},
		{"https://vendor.invalid/proxy%2Fv1/", "https://vendor.invalid/proxy%2Fv1/responses"},
		{"https://vendor.invalid/v1/%72esponses/", "https://vendor.invalid/v1/%72esponses/responses"},
		{"https://vendor.invalid/v1/responses/", "https://vendor.invalid/v1/responses"},
		{"https://vendor.invalid/proxy/v1?mode=x", "https://vendor.invalid/proxy/v1/responses?mode=x"},
	} {
		base, err := url.Parse(tc.base)
		if err != nil {
			t.Fatal(err)
		}
		endpoint, err := provider.EndpointURL(base, "/responses")
		if err != nil {
			t.Fatal(err)
		}
		if got := endpoint.String(); got != tc.want {
			t.Fatalf("got %s want %s", got, tc.want)
		}
		if base.String() != tc.base {
			t.Fatal("changed shared base")
		}
	}
}
