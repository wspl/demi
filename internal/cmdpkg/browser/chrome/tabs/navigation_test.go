package tabs

import (
	"testing"
)

func TestLiteralBrowserURLGlobs(t *testing.T) {
	for _, test := range []struct {
		pattern, url string
		match        bool
	}{
		{"/a/*", "/a/b/c", false}, {"/a/**", "/a/b/c", true}, {"/a/?", "/a/b", false}, {"/a/?", "/a/?", true}, {"/[a]{b}", "/[a]{b}", true}, {"/[a]{b}", "/ab", false},
		{"https://example.test/文/*", "https://example.test/文/a", true},
	} {
		t.Run(test.pattern+test.url, func(t *testing.T) {
			matcher, err := urlPattern(test.pattern)
			if err != nil {
				t.Fatal(err)
			}
			if got := matcher.MatchString(test.url); got != test.match {
				t.Fatalf("matched %v, want %v", got, test.match)
			}
		})
	}
}

func TestNavigationURLAdmission(t *testing.T) {
	for _, test := range []struct {
		url     string
		allowed bool
	}{
		{"http://localhost/", true}, {"https://example.test", true}, {"file:///tmp/page.html", true}, {"about:blank", true},
		{"javascript:alert(1)", false}, {"data:text/html,hi", false}, {"about:blank#fragment", false}, {"/relative", false},
	} {
		t.Run(test.url, func(t *testing.T) {
			if err := ValidateURL(test.url); (err == nil) != test.allowed {
				t.Fatalf("allowed %v: %v", test.allowed, err)
			}
		})
	}
}
