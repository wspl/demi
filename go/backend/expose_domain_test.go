package backend_test

import (
	"testing"

	"github.com/wspl/demi/go/backend"
)

// DEMI_EXPOSE_DOMAIN is a DNS name, read as a URL's host is, and an expose's
// hostname is one label under it in any case.
// Cost: parsing only.
func TestAnExposeDomainIsADomainNameAndAHostnameOneLabelUnderIt(t *testing.T) {
	for text, want := range map[string]string{
		"expose.demi.example": "expose.demi.example",
		"Expose.Demi.EXAMPLE": "expose.demi.example",
		"exposé.example":      "xn--expos-fsa.example",
		"expose.localhost":    "expose.localhost",
	} {
		domain, err := backend.ParseExposeDomain(text)
		if err != nil || domain.String() != want {
			t.Errorf("%q: %q, %v", text, domain, err)
		}
	}
	for _, text := range []string{"", "127.0.0.1", "[::1]", "expose..example", "expose.example.", "expose.example/x", "expose.example:8080", "user@expose.example", "expose example"} {
		if domain, err := backend.ParseExposeDomain(text); err == nil {
			t.Errorf("%q was accepted as %q", text, domain)
		}
	}
	domain, err := backend.ParseExposeDomain("expose.demi.example")
	if err != nil {
		t.Fatal(err)
	}
	if label, ok := domain.Label("K7X2.Expose.Demi.Example"); !ok || label != "k7x2" {
		t.Errorf("the label is %q, %v", label, ok)
	}
	for _, host := range []string{"expose.demi.example", "a.b.expose.demi.example", "k7x2.other.example", "k7x2expose.demi.example"} {
		if label, ok := domain.Label(host); ok {
			t.Errorf("%s has the label %q", host, label)
		}
	}
}
