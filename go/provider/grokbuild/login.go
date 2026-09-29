package grokbuild

import (
	"context"
	"fmt"
	"math"
	"net/http"
	"net/url"
	"time"
	"unicode"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/provider"
)

const ClientID = "b1a00492-073a-47ea-816f-4c329264a828"
const Scope = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write"

type accountKit struct{ p *Provider }

func (*accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true}
}
func (*accountKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAddUnsupported
}

//demi:wire open
type deviceCode struct {
	DeviceCode              provider.Secret        `json:"device_code" check:"func=provider.Validate"`
	UserCode                string                 `json:"user_code" check:"func=userCode"`
	VerificationURI         *string                `json:"verification_uri,omitzero" check:"nullabsent,func=verificationURI"`
	VerificationURIComplete *string                `json:"verification_uri_complete,omitzero" check:"nullabsent,func=verificationURI"`
	Interval                *provider.PollInterval `json:"interval,omitzero" check:"nullabsent,func=provider.Validate"`
}

func userCode(text string) error {
	if text != "" {
		valid := true
		for _, r := range text {
			if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '-') {
				valid = false
			}
		}
		if valid {
			return nil
		}
	}
	return &wire.InvalidError{Rule: "a user code is letters, digits and dashes"}
}
func verificationURI(text string) error {
	valid := true
	for _, r := range text {
		if unicode.IsControl(r) {
			valid = false
		}
	}
	u, err := url.Parse(text)
	if valid && err == nil && u.Host != "" && (u.Scheme == "https" || u.Scheme == "http" && (u.Hostname() == "localhost" || u.Hostname() == "127.0.0.1")) {
		return nil
	}
	return &wire.InvalidError{Rule: "a verification address is https, or http on localhost"}
}

//demi:wire open
type tokenRefusal struct {
	Error *provider.ReportedString `json:"error,omitzero" check:"nullabsent,func=provider.Validate"`
}

func (k *accountKit) oauth(ctx context.Context, path string, form url.Values) (*http.Response, error) {
	endpoint, err := provider.EndpointURL(&k.p.config.IssuerURL, path)
	if err != nil {
		return nil, err
	}
	response, err := send(ctx, k.p.http, http.MethodPost, endpoint.String(), http.Header{"Content-Type": {"application/x-www-form-urlencoded"}, "X-Grok-Client-Version": {ClientVersion}, "X-Grok-Client-Surface": {"ui"}}, []byte(form.Encode()))
	if err != nil {
		return nil, provider.TransportFailure("Grok sign-in", err)
	}
	return response, nil
}

// Login retains Grok's fractional intervals and waits after each response.
// oauth2.DeviceAccessToken only accepts integral seconds and polls on a ticker,
// which can poll immediately after a slow response instead of waiting again.
func (k *accountKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	response, err := k.oauth(ctx, "/oauth2/device/code", url.Values{"client_id": {ClientID}, "scope": {Scope}, "referrer": {"grok-build"}})
	if err != nil {
		return provider.NewAccount{}, err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return provider.NewAccount{}, fmt.Errorf("Grok device code request failed with HTTP %d", response.StatusCode)
	}
	device, err := provider.DecodeOAuthResponse(response, decode[deviceCode])
	if err != nil {
		return provider.NewAccount{}, err
	}
	uri := device.VerificationURIComplete
	if uri == nil {
		uri = device.VerificationURI
	}
	if uri == nil {
		return provider.NewAccount{}, fmt.Errorf("Grok device code failed: the response names no verification_uri")
	}
	deadline := time.Now().Add(provider.DeviceLoginLifetime)
	expires, _ := core.TimestampFromMillisecond(k.p.clock.Now().Millisecond() + 600000)
	pending(core.LoginPending{VerificationURL: *uri, UserCode: &device.UserCode, ExpiresAt: &expires})
	seconds := 5.0
	if device.Interval != nil {
		seconds = device.Interval.Seconds()
	}
	seconds = max(1, seconds)
	for {
		interval := time.Duration(min(seconds*float64(time.Second), float64(math.MaxInt64)))
		if interval < 0 {
			interval = time.Duration(math.MaxInt64)
		}
		timer := time.NewTimer(interval)
		select {
		case <-ctx.Done():
			timer.Stop()
			return provider.NewAccount{}, ctx.Err()
		case <-timer.C:
		}
		if !time.Now().Before(deadline) {
			return provider.NewAccount{}, fmt.Errorf("Grok device login timed out before the user confirmed")
		}
		response, err := k.oauth(ctx, "/oauth2/token", url.Values{"grant_type": {"urn:ietf:params:oauth:grant-type:device_code"}, "device_code": {device.DeviceCode.Expose()}, "client_id": {ClientID}})
		if err != nil {
			return provider.NewAccount{}, err
		}
		if response.StatusCode >= 200 && response.StatusCode < 300 {
			tokens, err := provider.DecodeOAuthResponse(response, decode[tokens])
			if err != nil {
				return provider.NewAccount{}, err
			}
			secret := k.secret(ctx, tokens)
			if ctx.Err() != nil {
				return provider.NewAccount{}, ctx.Err()
			}
			raw, err := secret.JSON()
			return provider.NewAccount{Secret: string(raw), Label: secret.label()}, err
		}
		refusal, _ := provider.DecodeOAuthResponse(response, decode[tokenRefusal])
		reason := reported(refusal.Error)
		if reason == nil {
			return provider.NewAccount{}, fmt.Errorf("Grok device login failed: HTTP %d", response.StatusCode)
		}
		switch *reason {
		case "slow_down":
			seconds += 5
		case "authorization_pending":
		default:
			return provider.NewAccount{}, fmt.Errorf("Grok device login failed: %s", *reason)
		}
	}
}

//demi:wire open
type userAnswer struct {
	UserID             *provider.ReportedString `json:"userId,omitzero" check:"nullabsent,func=provider.Validate"`
	UserIDSnake        *provider.ReportedString `json:"user_id,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalType      *provider.ReportedString `json:"principalType,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalTypeSnake *provider.ReportedString `json:"principal_type,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalID        *provider.ReportedString `json:"principalId,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalIDSnake   *provider.ReportedString `json:"principal_id,omitzero" check:"nullabsent,func=provider.Validate"`
	Email              *provider.ReportedString `json:"email,omitzero" check:"nullabsent,func=provider.Validate"`
}

func nonempty(v *string) *string {
	if v != nil && *v == "" {
		return nil
	}
	return v
}
func (k *accountKit) secret(ctx context.Context, tokens tokens) SecretDocument {
	id := claims{}
	if tokens.IDToken != nil {
		id = claimsOf(*tokens.IDToken)
	}
	principal := claimsOf(tokens.AccessToken).principal()
	user, email := reported(id.Sub), reported(id.Email)
	if principal != nil && (principal.Kind == "Team" || principal.Kind == "Organization") {
		user = &principal.ID
		email = nil
	}
	secret := SecretDocument{AccessToken: tokens.AccessToken, RefreshToken: tokens.RefreshToken, ExpiresAt: tokenExpiry(tokens, k.p.clock.Now()), Issuer: k.p.config.IssuerURL.String(), ClientID: ClientID, Principal: principal, UserID: nonempty(user), Email: nonempty(email)}
	headers := http.Header{"Authorization": {tokens.AccessToken.Bearer()}, "X-Xai-Token-Auth": {"xai-grok-cli"}, "X-Grok-Client-Version": {ClientVersion}, "X-Grok-Client-Mode": {"interactive"}}
	response, err := send(ctx, k.p.http, http.MethodGet, k.p.userURL, headers, nil)
	if err != nil {
		return secret
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return secret
	}
	details, err := provider.DecodeOAuthResponse(response, decode[userAnswer])
	if err != nil {
		return secret
	}
	user = nonempty(reported(details.UserID))
	if user == nil {
		user = nonempty(reported(details.UserIDSnake))
	}
	if user == nil {
		return secret
	}
	secret.UserID = user
	kind, idValue := nonempty(reported(details.PrincipalType)), nonempty(reported(details.PrincipalID))
	if kind == nil {
		kind = nonempty(reported(details.PrincipalTypeSnake))
	}
	if idValue == nil {
		idValue = nonempty(reported(details.PrincipalIDSnake))
	}
	if kind != nil && idValue != nil {
		secret.Principal = &Principal{Kind: *kind, ID: *idValue}
	}
	if email := nonempty(reported(details.Email)); email != nil {
		secret.Email = email
	}
	return secret
}
