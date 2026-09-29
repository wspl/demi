package backend_test

import (
	"testing"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/core"
)

// An installer names the backend by a plain http or https URL: a user, a
// password, a query or a fragment, even an empty one, would reach every
// runner that installs from it.
// Cost: parsing only.
func TestAnInstallersBackendURLIsAPlainHTTPURL(t *testing.T) {
	for text, want := range map[string]bool{
		"http://127.0.0.1:3271":        true,
		"https://demi.example/base/":   true,
		"http://@demi.example/":        true,
		"ws://demi.example":            false,
		"https://user@demi.example":    false,
		"https://:secret@demi.example": false,
		"https://demi.example/?":       false,
		"https://demi.example/?a=1":    false,
		"https://demi.example/#":       false,
		"https://demi.example/#top":    false,
	} {
		address, err := core.ParseURL(text)
		if err != nil {
			t.Fatalf("%s: %v", text, err)
		}
		if got := backend.InstallableURL(address); got != want {
			t.Errorf("%s: %v, want %v", text, got, want)
		}
	}
}
