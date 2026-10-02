package grokbuild

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

const clientID = "b1a00492-073a-47ea-816f-4c329264a828"
const scope = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write"

type loginKit struct {
	http            *http.Client
	issuer, userURL *url.URL
	clock           core.Clock
}

func (*loginKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true}
}
func (*loginKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAccountsUnsupported
}

type deviceAnswer struct {
	Device   provider.Secret       `json:"device_code"`
	Code     userCode              `json:"user_code"`
	URI      *verificationURI      `json:"verification_uri"`
	Complete *verificationURI      `json:"verification_uri_complete"`
	Interval provider.PollInterval `json:"interval" wire:"optional"`
}
type userCode string

func (c *userCode) UnmarshalJSON(data []byte) error {
	s, err := provider.DecodeUntagged[string](string(data))
	if err != nil {
		return err
	}
	if s == "" {
		return errors.New("a user code is letters, digits and dashes")
	}
	for _, r := range s {
		valid := r >= 'A' && r <= 'Z' || r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '-'
		if !valid {
			return errors.New("a user code is letters, digits and dashes")
		}
	}
	*c = userCode(s)
	return nil
}

type verificationURI string

func (v *verificationURI) UnmarshalJSON(data []byte) error {
	s, err := provider.DecodeUntagged[string](string(data))
	if err != nil {
		return err
	}
	canonical, err := parseIssuer(s)
	if err != nil {
		return errors.New("a verification address is https, or http on localhost")
	}
	u, err := url.Parse(string(canonical))
	if err != nil {
		return errors.New("a verification address is https, or http on localhost")
	}
	browsable := u.Scheme == "https" || u.Scheme == "http" && (u.Hostname() == "localhost" || u.Hostname() == "127.0.0.1")
	if u.Host == "" || strings.ContainsFunc(s, unicode.IsControl) || !browsable {
		return errors.New("a verification address is https, or http on localhost")
	}
	*v = verificationURI(s)
	return nil
}

//nolint:staticcheck // ST1005: user-facing messages are copied verbatim from Rust.
func (k *loginKit) post(ctx context.Context, path string, form url.Values) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, provider.EndpointURL(k.issuer, path).String(), strings.NewReader(form.Encode()))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("x-grok-client-version", clientVersion)
	req.Header.Set("x-grok-client-surface", "ui")
	response, err := k.http.Do(req)
	if err != nil {
		return nil, fmt.Errorf("Grok sign-in request failed: %w", withoutURL(err))
	}
	return response, nil
}
func (k *loginKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	result, err := k.login(ctx, pending)
	if err != nil {
		return provider.NewAccount{}, &provider.LoginError{Err: err}
	}
	return result, nil
}

//nolint:staticcheck // ST1005: user-facing messages are copied verbatim from Rust.
func (k *loginKit) login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	response, err := k.post(ctx, "/oauth2/device/code", url.Values{"client_id": {clientID}, "scope": {scope}, "referrer": {"grok-build"}})
	if err != nil {
		return provider.NewAccount{}, err
	}
	defer func() { _ = response.Body.Close() }() // The response is consumed or abandoned; close errors cannot change its result.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return provider.NewAccount{}, fmt.Errorf("Grok device code request failed with HTTP %d", response.StatusCode)
	}
	device, err := provider.DecodeJSONResponse(ctx, response, func(b []byte) (deviceAnswer, error) { return provider.DecodeUntagged[deviceAnswer](string(b)) })
	if err != nil {
		return provider.NewAccount{}, fmt.Errorf("Grok device code failed: %w", err)
	}
	verification := device.Complete
	if verification == nil {
		verification = device.URI
	}
	if verification == nil {
		return provider.NewAccount{}, errors.New("Grok device code failed: the response names no verification_uri")
	}
	pollCtx, cancel := context.WithTimeout(ctx, provider.DeviceLoginLifetime)
	defer cancel()
	code := string(device.Code)
	pending(core.LoginPending{VerificationURL: string(*verification), UserCode: &code, ExpiresAt: tokenExpiry(k.clock.Now(), provider.DeviceLoginLifetime)})
	answer, err := k.poll(pollCtx, device)
	pollErr := pollCtx.Err()
	cancel() // Confirmation has ended; enrichment uses the caller's context.
	if errors.Is(pollErr, context.DeadlineExceeded) {
		return provider.NewAccount{}, errors.New("Grok device login timed out before the user confirmed")
	}
	if err != nil {
		return provider.NewAccount{}, err
	}
	storedIssuer, err := parseIssuer(k.issuer.String())
	if err != nil {
		return provider.NewAccount{}, errors.New("an issuer is an http or https URL")
	}
	s := k.secret(ctx, answer, storedIssuer)
	if err := ctx.Err(); err != nil {
		return provider.NewAccount{}, err
	}
	encoded, err := s.MarshalJSON()
	if err != nil {
		return provider.NewAccount{}, err
	}
	return provider.NewAccount{Label: s.label(), Secret: string(encoded)}, nil
}

//nolint:staticcheck // ST1005: user-facing messages are copied verbatim from Rust.
func (k *loginKit) poll(ctx context.Context, device deviceAnswer) (tokens, error) {
	deadline, _ := ctx.Deadline() // login always supplies the ten-minute deadline.
	interval := max(device.Interval.Duration(), time.Second)
	for {
		timer := time.NewTimer(interval)
		select {
		case <-ctx.Done():
			timer.Stop()
			return tokens{}, ctx.Err()
		case <-timer.C:
		}
		if !time.Now().Before(deadline) {
			return tokens{}, errors.New("Grok device login timed out before the user confirmed")
		}
		response, err := k.post(ctx, "/oauth2/token", url.Values{"grant_type": {"urn:ietf:params:oauth:grant-type:device_code"}, "device_code": {device.Device.Expose()}, "client_id": {clientID}})
		if err != nil {
			return tokens{}, err
		}
		if response.StatusCode >= 200 && response.StatusCode < 300 {
			answer, err := provider.DecodeJSONResponse(ctx, response, decodeTokens)
			if err != nil {
				return tokens{}, fmt.Errorf("Grok device token failed: %w", err)
			}
			return answer, nil
		}
		refusal, _ := provider.DecodeJSONResponse(ctx, response, func(b []byte) (tokenRefusal, error) {
			return provider.DecodeUntagged[tokenRefusal](string(b))
		}) // An unreadable refusal is reported by its HTTP status.

		if refusal.Error.Value == nil {
			return tokens{}, fmt.Errorf("Grok device login failed: HTTP %d", response.StatusCode)
		}
		switch *refusal.Error.Value {
		case "slow_down":
			interval += 5 * time.Second
		case "authorization_pending":
		default:
			return tokens{}, fmt.Errorf("Grok device login failed: %s", *refusal.Error.Value)
		}
	}
}

type tokenRefusal struct {
	Error provider.ReportedString `json:"error" wire:"optional"`
}

type userAnswer struct {
	User      provider.Reported[provider.NonEmpty] `json:"userId" wire:"optional"`
	UserSnake provider.Reported[provider.NonEmpty] `json:"user_id" wire:"optional"`
	Kind      provider.Reported[provider.NonEmpty] `json:"principalType" wire:"optional"`
	KindSnake provider.Reported[provider.NonEmpty] `json:"principal_type" wire:"optional"`
	ID        provider.Reported[provider.NonEmpty] `json:"principalId" wire:"optional"`
	IDSnake   provider.Reported[provider.NonEmpty] `json:"principal_id" wire:"optional"`
	Email     provider.Reported[provider.NonEmpty] `json:"email" wire:"optional"`
}

func (k *loginKit) secret(ctx context.Context, t tokens, storedIssuer issuer) (s secret) {
	var id claims
	if t.ID != nil {
		id = tokenClaims(*t.ID)
	}
	acting := tokenClaims(t.Access).principal()
	s = secret{AccessToken: t.Access, RefreshToken: t.Refresh, Issuer: storedIssuer, ClientID: clientID, UserID: id.Sub.Value, Email: id.Email.Value, Principal: acting}
	// A token's team or organization remains authoritative after user enrichment.
	defer func() {
		if acting != nil && (acting.Kind == "Team" || acting.Kind == "Organization") {
			s.Principal = acting
			s.UserID = &acting.ID
			s.Email = nil
		}
	}()
	if s.UserID != nil && *s.UserID == "" {
		s.UserID = nil
	}
	if s.Email != nil && *s.Email == "" {
		s.Email = nil
	}
	if t.Lifetime.Duration != nil {
		s.ExpiresAt = tokenExpiry(k.clock.Now(), *t.Lifetime.Duration)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, k.userURL.String(), nil)
	if err != nil {
		return s
	}
	req.Header.Set("Authorization", s.AccessToken.Bearer().Expose())
	req.Header.Set("x-xai-token-auth", "xai-grok-cli")
	req.Header.Set("x-grok-client-version", clientVersion)
	req.Header.Set("x-grok-client-mode", "interactive")
	response, err := k.http.Do(req)
	if err != nil {
		return s
	}
	defer func() { _ = response.Body.Close() }() // The response is consumed or abandoned; close errors cannot change its result.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return s
	}
	user, err := provider.DecodeJSONResponse(ctx, response, func(b []byte) (userAnswer, error) { return provider.DecodeUntagged[userAnswer](string(b)) })
	if err != nil {
		return s
	}
	uid := user.User.Value
	if uid == nil {
		uid = user.UserSnake.Value
	}
	if uid == nil {
		return s
	}
	value := string(*uid)
	s.UserID = &value
	kind, pid := user.Kind.Value, user.ID.Value
	if kind == nil {
		kind = user.KindSnake.Value
	}
	if pid == nil {
		pid = user.IDSnake.Value
	}
	if kind != nil && pid != nil {
		s.Principal = &principal{Kind: string(*kind), ID: string(*pid)}
	}
	if user.Email.Value != nil {
		email := string(*user.Email.Value)
		s.Email = &email
	}
	return s
}
