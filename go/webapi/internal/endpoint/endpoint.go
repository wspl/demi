// Package endpoint checks and serializes provider HTTP endpoints.
package endpoint

import (
	"errors"

	"github.com/wspl/demi/go/core"
)

// Parse keeps the WHATWG parser's serialization of an HTTP(S) URL with a host.
func Parse(text string) (string, error) {
	value, err := core.ParseURL(text)
	if err != nil || (value.Scheme() != "http" && value.Scheme() != "https") || value.Hostname() == "" {
		return "", errors.New("must be an http or https URL")
	}
	return value.String(), nil
}
