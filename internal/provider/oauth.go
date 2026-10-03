package provider

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/wspl/demi/internal/contract"
)

// DeviceLoginLifetime bounds a device login to ten minutes.
const DeviceLoginLifetime = 10 * time.Minute

// DefaultPollInterval is RFC 8628's interval without a usable server preference.
const DefaultPollInterval = 5 * time.Second

var decimalSeconds = regexp.MustCompile(`^[0-9]+(\.[0-9]+)?$`)

// OAuthSeconds is a finite, nonnegative duration reported as a number or digits.
type OAuthSeconds float64

// UnmarshalJSON accepts numeric seconds or a decimal string.
func (s *OAuthSeconds) UnmarshalJSON(data []byte) error {
	text := string(data)
	if len(data) > 0 && data[0] == '"' {
		if err := json.Unmarshal(data, &text); err != nil {
			return errors.New("expected a number of seconds")
		}
		text = strings.TrimSpace(text)
		if !decimalSeconds.MatchString(text) {
			return errors.New("expected a number of seconds")
		}
	} else if !json.Valid(data) {
		return errors.New("expected a number of seconds")
	}
	value, err := strconv.ParseFloat(text, 64)
	if err != nil || math.IsNaN(value) || math.IsInf(value, 0) || value < 0 {
		return errors.New("expected a number of seconds")
	}
	*s = OAuthSeconds(value)
	return nil
}

// Duration returns a representable Go duration, refusing overflow instead of panicking.
func (s OAuthSeconds) Duration() (time.Duration, error) {
	nanos := math.Round(float64(s) * 1e9)
	if nanos < 0 || nanos >= float64(math.MaxInt64) || math.IsInf(nanos, 0) || math.IsNaN(nanos) {
		return 0, errors.New("OAuth duration outside Go duration range")
	}
	return time.Duration(nanos), nil
}

// PollInterval holds a server's polling interval; its zero value uses five seconds.
type PollInterval struct{ value *time.Duration }

// Duration returns the preference or RFC 8628's default.
func (p PollInterval) Duration() time.Duration {
	if p.value == nil {
		return DefaultPollInterval
	}
	return *p.value
}

// UnmarshalJSON reads a usable preference or restores the default.
func (p *PollInterval) UnmarshalJSON(data []byte) error {
	p.value = nil
	var seconds OAuthSeconds
	if err := seconds.UnmarshalJSON(data); err == nil {
		duration, err := seconds.Duration()
		if err == nil {
			p.value = &duration
		}
	}
	return nil
}

// Lifetime is a device code or token's positive lifetime; nil means absent.
type Lifetime struct{ Duration *time.Duration }

// UnmarshalJSON reads a positive lifetime, treating unusable values as absent.
func (l *Lifetime) UnmarshalJSON(data []byte) error {
	l.Duration = nil
	var seconds OAuthSeconds
	if err := seconds.UnmarshalJSON(data); err == nil && seconds > 0 {
		duration, err := seconds.Duration()
		if err == nil {
			l.Duration = &duration
		}
	}
	return nil
}

// SecretFault distinguishes malformed JSON from a wrong document shape.
type SecretFault uint8

// Secret decoder failure categories, without credential values.
const (
	// SecretSyntax identifies malformed JSON in a secret document.
	SecretSyntax SecretFault = iota
	// SecretShape identifies an invalid secret document shape.
	SecretShape
)

// SecretDecodeError reports a path and kind without ever quoting secret values.
type SecretDecodeError struct {
	Path  string
	Fault SecretFault
}

// Error returns the diagnostic for this failure.
func (e *SecretDecodeError) Error() string {
	reason := "not JSON"
	if e.Fault == SecretShape {
		reason = "a field is missing, unknown or of the wrong type"
	}
	return "malformed at " + e.Path + ": " + reason
}

// DecodeSecretDocument wraps a family's schema decoder, removing value-bearing error text.
// Stored Demi documents pass their generated Decode function; token replies pass
// a vendor decoder that declares only the fields used by the family.
func DecodeSecretDocument[T any](text string, decode func([]byte) (T, error)) (T, error) {
	var zero T
	if !json.Valid([]byte(text)) {
		return zero, &SecretDecodeError{Path: ".", Fault: SecretSyntax}
	}
	value, err := decode([]byte(text))
	if err == nil {
		return value, nil
	}
	path := "."
	var field *contract.Error
	var wire *WireError
	if errors.As(err, &field) && field.Path != "" {
		path = field.Path
		// A missing top-level member is reported at the object ("."), not at
		// the absent member. Inspect presence without quoting any value.
		var object map[string]json.RawMessage
		if json.Unmarshal([]byte(text), &object) == nil && object != nil && !strings.ContainsAny(path, ".[ ") {
			if _, present := object[path]; !present {
				path = "."
			}
		}
	}
	if errors.As(err, &wire) {
		path = wire.Field
	}
	return zero, &SecretDecodeError{Path: path, Fault: SecretShape}
}

// DecodeJSONResponse decodes and closes a token response using its schema decoder.
func DecodeJSONResponse[T any](
	ctx context.Context,
	response *http.Response,
	decode func([]byte) (T, error),
) (T, error) {
	var zero T
	stop := closeOnCancel(ctx, response.Body)
	defer stop()
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return zero, errors.New("the response body could not be read")
	}
	value, err := DecodeSecretDocument(string(data), decode)
	if err != nil {
		return zero, fmt.Errorf("the response is %w", err)
	}
	return value, nil
}

// JWTClaims reads identity or expiry claims without verifying the signature.
// The supplied decoder determines which claims are read and validated.
func JWTClaims[T any](token string, decode func([]byte) (T, error)) (T, bool) {
	var zero T
	segments := strings.Split(token, ".")
	if len(segments) != 3 || segments[1] == "" {
		return zero, false
	}
	payload := segments[1]
	// Padding is optional, but any supplied padding must be canonical.
	encoding := base64.RawURLEncoding.Strict()
	if strings.Contains(payload, "=") {
		encoding = base64.URLEncoding.Strict()
	}
	data, err := encoding.DecodeString(payload)
	if err != nil || !json.Valid(data) {
		return zero, false
	}
	value, err := decode(data)
	if err != nil {
		return zero, false
	}
	return value, true
}
