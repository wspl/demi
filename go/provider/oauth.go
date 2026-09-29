package provider

import (
	"context"
	"encoding/base64"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// DeviceLoginLifetime is how long a device login waits for its user to
// confirm, from the moment its code is shown, whatever the vendor's code
// allows (providers.md § Subscription accounts).
const DeviceLoginLifetime = 10 * time.Minute

// errDeviceLoginExpired is the cause of a device login's wait once its
// lifetime has passed.
var errDeviceLoginExpired = errors.New("device login expired")

// DeviceLoginWait returns ctx limited to DeviceLoginLifetime from now, for
// the polls and the pauses of a device login whose code was just shown: a poll
// in flight or a vendor's long interval ends when the lifetime does.
func DeviceLoginWait(ctx context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeoutCause(ctx, DeviceLoginLifetime, errDeviceLoginExpired)
}

// DeviceLoginExpired reports whether wait, from DeviceLoginWait, ended because
// the login's lifetime passed rather than because its caller cancelled it.
func DeviceLoginExpired(wait context.Context) bool {
	return errors.Is(context.Cause(wait), errDeviceLoginExpired)
}

var oauthDigits = regexp.MustCompile(`^[0-9]+(\.[0-9]+)?$`)

// OAuthSeconds accepts the number or digits an OAuth server reports.
//
//demi:opaque
type OAuthSeconds struct{ Seconds float64 }

func (s *OAuthSeconds) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	value, err := dec.ReadValue()
	if err != nil {
		return err
	}
	var n float64
	switch value.Kind() {
	case '"':
		var text string
		if err := json.Unmarshal(value, &text); err != nil {
			return err
		}
		text = strings.TrimSpace(text)
		if !oauthDigits.MatchString(text) {
			return errors.New("expected a number of seconds")
		}
		n, err = strconv.ParseFloat(text, 64)
	case '0':
		err = json.Unmarshal(value, &n)
	default:
		return errors.New("expected a number of seconds")
	}
	if err != nil || math.IsInf(n, 0) || math.IsNaN(n) || n < 0 {
		return errors.New("expected a number of seconds")
	}
	s.Seconds = n
	return nil
}

// JWTClaims reads unverified claims. The vendor verifies the signature; decode
// is the family's boundary decoder for the identity and expiry fields it reads.
func JWTClaims[T any](token string, decode func([]byte) (T, error)) (T, bool) {
	var zero T
	parts := strings.Split(token, ".")
	if len(parts) != 3 || parts[1] == "" {
		return zero, false
	}
	encoding := base64.RawURLEncoding
	if strings.HasSuffix(parts[1], "=") {
		encoding = base64.URLEncoding
	}
	payload, err := encoding.DecodeString(parts[1])
	if err != nil {
		return zero, false
	}
	claims, err := decode(payload)
	if err != nil {
		return zero, false
	}
	return claims, true
}

// PollInterval uses the server preference or RFC 8628's five-second default.
// The zero value means no usable preference, including an absent field.
//
//demi:opaque
type PollInterval struct{ seconds *float64 }

func (p PollInterval) Seconds() float64 {
	if p.seconds == nil {
		return 5
	}
	return *p.seconds
}
func (p *PollInterval) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	value, err := dec.ReadValue()
	if err != nil {
		return err
	}
	var seconds OAuthSeconds
	p.seconds = nil
	if json.Unmarshal(value, &seconds) == nil {
		p.seconds = &seconds.Seconds
	}
	return nil
}

// Lifetime treats a nonpositive or unusable duration as absent.
//
//demi:opaque
type Lifetime struct{ Seconds *float64 }

func (l *Lifetime) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	value, err := dec.ReadValue()
	if err != nil {
		return err
	}
	var seconds OAuthSeconds
	l.Seconds = nil
	if json.Unmarshal(value, &seconds) == nil && seconds.Seconds > 0 {
		l.Seconds = &seconds.Seconds
	}
	return nil
}

// DecodeOAuthResponse consumes and closes a login or refresh response. The
// boundary decoder's errors are reduced to safe paths and fault categories.
func DecodeOAuthResponse[T any](response *http.Response, boundary func([]byte) (T, error)) (T, error) {
	defer response.Body.Close()
	var zero T
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return zero, errors.New("the response body could not be read")
	}
	value, err := DecodeSecret(data, boundary)
	if err != nil {
		return zero, fmt.Errorf("the response is %w", err)
	}
	return value, nil
}

func (PollInterval) validate() error { return nil }
func (Lifetime) validate() error     { return nil }
