package main

import (
	"strings"
	"testing"
)

// Cost: environment table only; no build, process or network.
func TestDevProviderNeedsAllFourVariables(t *testing.T) {
	complete := map[string]string{
		"DEMI_DEV_PROVIDER_BASE_URL":       "https://models.example.test/v1",
		"DEMI_DEV_PROVIDER_API_KEY":        "sk-development",
		"DEMI_DEV_PROVIDER_MODEL":          "vendor/model-flash",
		"DEMI_DEV_PROVIDER_CONTEXT_WINDOW": "1000000",
	}
	with := func(name, value string) func(string) string {
		return func(key string) string {
			if key == name {
				return value
			}
			return complete[key]
		}
	}
	if _, configured, err := readDevProvider(func(string) string {
		return ""
	}); configured || err != nil {
		t.Fatalf("none set: configured %v, error %v", configured, err)
	}
	p, configured, err := readDevProvider(with("", ""))
	if err != nil || !configured {
		t.Fatalf("all set: configured %v, error %v", configured, err)
	}
	if p.BaseURL != "https://models.example.test/v1" || p.APIKey != "sk-development" ||
		p.Model != "vendor/model-flash" || p.ContextWindow != 1000000 {
		t.Fatalf("all set: got %+v", p)
	}
	for _, tc := range []struct{ name, value string }{
		{"DEMI_DEV_PROVIDER_BASE_URL", ""},
		{"DEMI_DEV_PROVIDER_API_KEY", ""},
		{"DEMI_DEV_PROVIDER_MODEL", ""},
		{"DEMI_DEV_PROVIDER_CONTEXT_WINDOW", ""},
		{"DEMI_DEV_PROVIDER_BASE_URL", "not a url"},
		{"DEMI_DEV_PROVIDER_CONTEXT_WINDOW", "0"},
		{"DEMI_DEV_PROVIDER_CONTEXT_WINDOW", "-1"},
		{"DEMI_DEV_PROVIDER_CONTEXT_WINDOW", "abc"},
		{"DEMI_DEV_PROVIDER_CONTEXT_WINDOW", "4294967296"},
	} {
		_, configured, err := readDevProvider(with(tc.name, tc.value))
		if err == nil || configured || !strings.Contains(err.Error(), tc.name) {
			t.Errorf("%s=%q: configured %v, error %v; want an error naming it", tc.name, tc.value, configured, err)
		}
	}
}
