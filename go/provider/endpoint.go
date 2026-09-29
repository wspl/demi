package provider

import (
	"errors"
	"net/url"
	"strings"
)

// EndpointURL appends a request path unless the configured API base already
// ends in that path. The query and fragment remain those of the base.
func EndpointURL(base *url.URL, path string) (*url.URL, error) {
	result := *base
	trimmed := strings.TrimRight(result.EscapedPath(), "/")
	if !strings.HasSuffix(trimmed, path) {
		trimmed += path
	}
	decoded, err := url.PathUnescape(trimmed)
	if err != nil {
		return nil, errors.New("invalid provider request path escaping")
	}
	result.Path = decoded
	result.RawPath = trimmed
	return &result, nil
}

// HeaderText checks vendor identity fields before sending them as HTTP headers.
// net/http has no exported field-value validator.
func HeaderText(text string) bool {
	for _, b := range []byte(text) {
		if b == '\r' || b == '\n' || b == 127 || b < 32 && b != '\t' {
			return false
		}
	}
	return true
}
