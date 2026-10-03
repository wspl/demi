package provider_test

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestOAuthDurations(t *testing.T) {
	for _, tc := range []struct {
		text    string
		seconds float64
	}{{"5", 5}, {`"28800"`, 28800}, {`" 1.5 "`, 1.5}, {"0", 0}} {
		var seconds provider.OAuthSeconds
		if err := json.Unmarshal([]byte(tc.text), &seconds); err != nil {
			t.Fatal(err)
		}
		requireEqual(t, float64(seconds), tc.seconds)
	}
	for _, text := range []string{`"1e3"`, `"soon"`, `-1`, `true`, `null`, `"-5"`} {
		var seconds provider.OAuthSeconds
		if err := json.Unmarshal([]byte(text), &seconds); err == nil {
			t.Fatalf("accepted %s", text)
		}
	}
	type answer struct {
		Interval  provider.PollInterval `json:"interval"   wire:"optional"`
		ExpiresIn provider.Lifetime     `json:"expires_in" wire:"optional"`
	}
	for _, tc := range []struct {
		text     string
		interval time.Duration
	}{{
		`{}`,
		5 * time.Second,
	}, {
		`{"interval":"soon"}`,
		5 * time.Second,
	}, {
		`{"interval":"0"}`,
		0,
	}, {
		`{"interval":3}`,
		3 * time.Second,
	}} {
		value, err := provider.DecodeUntagged[answer](tc.text)
		if err != nil {
			t.Fatal(err)
		}
		requireEqual(t, value.Interval.Duration(), tc.interval)
	}
	value, err := provider.DecodeUntagged[answer](`{"expires_in":"600"}`)
	if err != nil || value.ExpiresIn.Duration == nil || *value.ExpiresIn.Duration != 600*time.Second {
		t.Fatalf("%+v %v", value, err)
	}
	for _, text := range []string{"0", `"forever"`, `-3`} {
		value, err := provider.DecodeUntagged[answer](`{"expires_in":` + text + `}`)
		if err != nil || value.ExpiresIn.Duration != nil {
			t.Fatalf("%+v %v", value, err)
		}
	}
}

func TestJWTClaimsWithoutSignature(t *testing.T) {
	decode := func(data []byte) (any, error) { return provider.DecodeUntagged[any](string(data)) }
	for _, value := range []any{
		map[string]any{
			"sub": "user-1",
			"exp": 1700000000,
		},
		map[string]any{
			"email": "zoé@example.com",
		},
		42,
	} {
		token := providertest.JWT(t, value)
		claims := provider.JWTClaims(token, decode)
		if claims == nil {
			t.Fatalf("no claims for %s", token)
		}
		requireEqual(t, *claims, jsonValue(t, encoded(t, value)))
	}
	requireEqual(t, *provider.JWTClaims("header.eyJhIjoxfQ==.signature", decode), jsonValue(t, `{"a":1}`))
	for _, text := range []string{"header.payload", "", "a..c", "header.!!!!.signature", "a.b.c.d", "header.bm9wZQ.sig"} {
		if provider.JWTClaims(text, decode) != nil {
			t.Fatalf("accepted %s", text)
		}
	}
	type claims struct {
		Sub string `json:"sub"`
	}
	if provider.JWTClaims(
		providertest.JWT(t, 42),
		func(data []byte) (claims, error) { return provider.DecodeUntagged[claims](string(data)) },
	) != nil {
		t.Fatal("accepted numeric claims as object")
	}
}

func TestUTF16CacheKey(t *testing.T) {
	requireEqual(t, provider.ShortHash(""), "811c9dc5")
	requireEqual(t, provider.ShortHash("a"), "e40c292c")
	for _, tc := range []struct{ text, want string }{
		{
			strings.Repeat("x", 64),
			strings.Repeat("x", 64),
		},
		{
			"chat-" +
				strings.Repeat("x", 80),
			"session_cc30bf2",
		},
		{
			strings.Repeat("😀", 33),
			"session_21a3538",
		},
		{
			strings.Repeat("会话", 40),
			"session_1bc4bf95",
		},
	} {
		requireEqual(t, provider.PromptCacheKey(tc.text), tc.want)
	}
}
