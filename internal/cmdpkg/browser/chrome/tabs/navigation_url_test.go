package tabs_test

import (
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

func TestNavigationURLAdmission(t *testing.T) {
	for _, test := range []struct {
		url     string
		allowed bool
	}{
		{"http://localhost/", true},
		{"https://example.test", true},
		{"file:///tmp/page.html", true},
		{"about:blank", true},
		{"javascript:alert(1)", false},
		{"data:text/html,hi", false},
		{"about:blank#fragment", false},
		{"/relative", false},
	} {
		t.Run(test.url, func(t *testing.T) {
			if err := tabs.ValidateURL(test.url); (err == nil) != test.allowed {
				t.Fatalf("allowed %v: %v", test.allowed, err)
			}
		})
	}
}
