package provider

import (
	"errors"
	"net/url"
)

// WithoutURL returns the cause of a *url.Error, keeping request URLs such as
// issuer and proxy addresses out of messages; any other error is returned as it is.
func WithoutURL(err error) error {
	var e *url.Error
	if errors.As(err, &e) {
		return e.Err
	}
	return err
}
