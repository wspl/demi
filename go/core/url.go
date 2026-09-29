package core

import (
	"errors"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ParseURL uses WHATWG parsing for product URLs, including provider endpoints
// and managed-runner backend addresses. Callers apply their scheme and host
// rules to the parsed URL; String returns the parser's serialization.
func ParseURL(text string) (*whatwg.Url, error) {
	value, err := whatwg.Parse(text)
	if err != nil {
		return nil, errors.New("must be an absolute URL")
	}
	return value, nil
}
