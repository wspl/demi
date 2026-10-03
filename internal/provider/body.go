package provider

import (
	"context"
	"fmt"
	"net/url"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// EndpointURL appends path unless base already ends with it, ignoring trailing slashes.
func EndpointURL(base *url.URL, path string) *url.URL {
	result := *base
	trimmed := strings.TrimRight(result.Path, "/")
	if !strings.HasSuffix(trimmed, path) {
		trimmed += path
	}
	result.Path = trimmed
	result.RawPath = ""
	return &result
}

// JSONBody encodes a request body with contract.EncodeJSON, which leaves <, >, &, U+2028 and U+2029 unescaped.
func JSONBody(body any) ([]byte, error) { return contract.EncodeJSON(body) }

// EncodeBody builds a request in the caller's goroutine, outside state locks.
// A failed body build becomes a run failure without a recovery code.
func EncodeBody[T any](ctx context.Context, label string, encode func() (T, error)) (value T, err error) {
	if err := ctx.Err(); err != nil {
		return value, err
	}
	defer func() {
		if recover() != nil {
			// A panic's value may contain request secrets; do not print it.
			err = &Failure{Message: label + " API request body was not built: encoder panicked"}
		}
	}()
	value, err = encode()
	if err != nil {
		return value, &Failure{Message: fmt.Sprintf("%s API request body was not built: %v", label, err)}
	}
	return value, nil
}
