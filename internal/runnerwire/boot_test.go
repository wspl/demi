package runnerwire_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerwire"
)

func TestBackendURL(t *testing.T) {
	for _, value := range []string{"https://demi.example.com", "http://10.0.0.5:3271", "wss://demi.example.com/runner"} {
		if _, err := runnerwire.ParseBackendURL(value); err != nil {
			t.Errorf("%s: %v", value, err)
		}
	}
	for _, value := range []string{
		"https://user:pass@demi.example.com",
		"https://user@demi.example.com",
		"https://demi.example.com/#runner",
		"https://demi.example.com/#",
		"ftp://demi.example.com",
		"file:///tmp/backend",
		"not a url",
	} {
		if _, err := runnerwire.ParseBackendURL(value); err == nil {
			t.Errorf("accepted %s", value)
		}
	}
	boot, err := runnerwire.DecodeManagedBoot(
		[]byte(`{"backendUrl":"HTTPS://DEMI.EXAMPLE.COM:443/a/../","deviceToken":"secret"}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	if boot.BackendURL.String() != "https://demi.example.com/" {
		t.Fatalf("normal form: %s", boot.BackendURL)
	}
	encoded, err := contract.EncodeJSON(boot)
	if err != nil {
		t.Fatal(err)
	}
	if string(encoded) != `{"backendUrl":"https://demi.example.com/","deviceToken":"secret"}` {
		t.Fatalf("boot JSON: %s", encoded)
	}
	for _, value := range []string{
		`{"backendUrl":"https://demi.example.com","deviceToken":""}`,
		`{"backendUrl":"https://demi.example.com","deviceToken":"a b"}`,
		`{"backendUrl":"https://demi.example.com","deviceToken":null}`,
		`{"backendUrl":"https://demi.example.com","deviceToken":"ok","extra":1}`,
		`{"backendUrl":"https://user@demi.example.com","deviceToken":"ok"}`,
	} {
		if _, err := runnerwire.DecodeManagedBoot([]byte(value)); err == nil {
			t.Errorf("accepted boot %s", value)
		}
	}
}

func TestDeviceToken(t *testing.T) {
	for _, tc := range []struct {
		value string
		valid bool
	}{
		{
			"",
			false,
		}, {
			"secret",
			true,
		}, {
			strings.Repeat("😀", 2048),
			true,
		}, {
			strings.Repeat("😀", 2049),
			false,
		}, {
			strings.Repeat("x", 4096),
			true,
		}, {
			strings.Repeat("x", 4097),
			false,
		}, {
			"a\u0085b",
			false,
		}, {
			"a\u2003b",
			false,
		}, {
			"a\ufeffb",
			true,
		},
	} {
		token, err := runnerwire.ParseDeviceToken(tc.value)
		if (err == nil) != tc.valid {
			t.Errorf("token length %d: %v", len(tc.value), err)
		}
		if err == nil && token.Expose() != tc.value {
			t.Fatal("credential changed")
		}
	}
	token, err := runnerwire.ParseDeviceToken("secret")
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"%v", "%+v", "%#v", "%s", "%q"} {
		if text := fmt.Sprintf(format, token); strings.Contains(text, "secret") {
			t.Fatalf("token leaked through %s", format)
		}
	}
}
