package core

import (
	"errors"
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ParseURL uses WHATWG parsing for product URLs, including provider endpoints
// and managed-runner backend addresses, as the Rust's url crate does. Callers
// apply their scheme and host rules to the parsed URL; String returns the
// parser's serialization.
func ParseURL(text string) (*whatwg.Url, error) {
	emptyLabel := false
	parser := whatwg.NewParser(whatwg.WithPreParseHostFunc(func(_ *whatwg.Url, host string) string {
		emptyLabel = emptyLabel || hasEmptyPunycodeLabel(host)
		return host
	}))
	value, err := parser.Parse(text)
	if err != nil || emptyLabel {
		return nil, errors.New("must be an absolute URL")
	}
	return value, nil
}

// hasEmptyPunycodeLabel reports whether host has a label that is the Punycode
// prefix alone. UTS 46 refuses it since Unicode 15.1, as the Rust's idna crate
// does, while golang.org/x/net/idna reads it as an empty label.
func hasEmptyPunycodeLabel(host string) bool {
	labels := strings.FieldsFunc(host, func(r rune) bool {
		// The full stop and the three dots UTS 46 maps to it.
		return r == '.' || r == '。' || r == '．' || r == '｡'
	})
	for _, label := range labels {
		if strings.EqualFold(label, "xn--") {
			return true
		}
	}
	return false
}
