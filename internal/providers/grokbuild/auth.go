package grokbuild

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

type claims struct {
	Exp       provider.Reported[float64] `json:"exp" wire:"optional"`
	Sub       provider.ReportedString    `json:"sub" wire:"optional"`
	Email     provider.ReportedString    `json:"email" wire:"optional"`
	Kind      provider.ReportedString    `json:"principal_type" wire:"optional"`
	KindCamel provider.ReportedString    `json:"principalType" wire:"optional"`
	ID        provider.ReportedString    `json:"principal_id" wire:"optional"`
	IDCamel   provider.ReportedString    `json:"principalId" wire:"optional"`
}

func tokenClaims(token provider.Secret) claims {
	value := provider.JWTClaims(token.Expose(), func(b []byte) (claims, error) { return provider.DecodeUntagged[claims](string(b)) })
	if value == nil {
		return claims{}
	}
	return *value
}
func (c claims) principal() *principal {
	kind, id := c.Kind.Value, c.ID.Value
	if kind == nil {
		kind = c.KindCamel.Value
	}
	if id == nil {
		id = c.IDCamel.Value
	}
	if kind == nil || id == nil {
		return nil
	}
	return &principal{Kind: *kind, ID: *id}
}
func (c claims) expiry() *core.Timestamp {
	if c.Exp.Value == nil {
		return nil
	}
	return provider.UnixSeconds(*c.Exp.Value)
}
func (s secret) label() provider.AccountLabel {
	identity := strings.TrimRight(string(s.Issuer), "/") + "::"
	switch {
	case s.UserID != nil:
		identity += *s.UserID
	case s.Email != nil:
		identity += *s.Email
	default:
		identity += s.ClientID
	}
	name := identity
	if s.UserID != nil {
		name = *s.UserID
	}
	if s.Email != nil {
		name = *s.Email
	}
	detail := "oidc"
	return provider.AccountLabel{Label: name, Detail: &detail, IdentityKey: &identity}
}
func (s secret) expiring(now core.Timestamp) bool {
	expiry := s.ExpiresAt
	if expiry == nil {
		expiry = tokenClaims(s.AccessToken).expiry()
	}
	if expiry == nil {
		return false
	}
	end, err := expiry.Time()
	if err != nil {
		return false
	}
	start, err := now.Time()
	return err == nil && end.Sub(start) <= 5*time.Minute
}

type auth struct {
	pool    provider.CredentialPool
	account *string
	clock   core.Clock
}

func (a *auth) document() (provider.AccountDocument, *provider.AuthFailure) {
	if a.account == nil {
		return nil, &provider.AuthFailure{Family: "Grok", Reason: provider.AuthReasonMissing}
	}
	return a.pool.Document(*a.account), nil
}
func (a *auth) stored(ctx context.Context) (secret, *provider.AuthFailure) {
	doc, failure := a.document()
	if failure != nil {
		return secret{}, failure
	}
	stored, err := provider.ReadSecret(ctx, doc, Decodesecret)
	if err != nil {
		f := provider.AccountAuthFailure("Grok", err)
		return secret{}, &f
	}
	return stored.Secret, nil
}
func (a *auth) credentials(ctx context.Context, client *http.Client, refused *provider.Secret) (secret, *provider.AuthFailure) {
	doc, failure := a.document()
	if failure != nil {
		return secret{}, failure
	}
	s, err := provider.Renew(ctx, doc, Decodesecret, func(s secret) bool {
		return s.RefreshToken != nil && ((refused != nil && s.AccessToken == *refused) || s.expiring(a.clock.Now()))
	}, func(ctx context.Context, s secret) (secret, error) { return a.refresh(ctx, client, s) })
	if err != nil {
		f := provider.AccountAuthFailure("Grok", err)
		return secret{}, &f
	}
	return s, nil
}

// A refresh reads no id_token; only login consumes that claim source.
type refreshedTokens struct {
	Access   provider.Secret   `json:"access_token"`
	Refresh  *provider.Secret  `json:"refresh_token"`
	Lifetime provider.Lifetime `json:"expires_in" wire:"optional"`
}

type tokens struct {
	Access   provider.Secret   `json:"access_token"`
	Refresh  *provider.Secret  `json:"refresh_token"`
	Lifetime provider.Lifetime `json:"expires_in" wire:"optional"`
	ID       *provider.Secret  `json:"id_token"`
}

func decodeTokens(data []byte) (tokens, error) { return provider.DecodeUntagged[tokens](string(data)) }

//nolint:staticcheck // ST1005: user-facing messages are copied verbatim from Rust.
func (a *auth) refresh(ctx context.Context, client *http.Client, s secret) (secret, error) {
	form := url.Values{"grant_type": {"refresh_token"}, "refresh_token": {s.RefreshToken.Expose()}, "client_id": {s.ClientID}}
	if s.Principal != nil {
		form.Set("principal_type", s.Principal.Kind)
		form.Set("principal_id", s.Principal.ID)
	}
	issuer, err := url.Parse(string(s.Issuer))
	if err != nil {
		return secret{}, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, provider.EndpointURL(issuer, "/oauth2/token").String(), strings.NewReader(form.Encode()))
	if err != nil {
		return secret{}, err
	}
	req.Header.Set("Accept", "application/json")
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	response, err := client.Do(req)
	if err != nil {
		return secret{}, fmt.Errorf("Grok token refresh failed: %w", withoutURL(err))
	}
	defer func() { _ = response.Body.Close() }() // The response is consumed or abandoned; close errors cannot change its result.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return secret{}, fmt.Errorf("Grok token refresh failed with HTTP %d", response.StatusCode)
	}
	answer, err := provider.DecodeJSONResponse(ctx, response, func(data []byte) (refreshedTokens, error) {
		return provider.DecodeUntagged[refreshedTokens](string(data))
	})
	if err != nil {
		return secret{}, fmt.Errorf("Grok token refresh failed: %w", err)
	}
	s.AccessToken = answer.Access
	if answer.Refresh != nil {
		s.RefreshToken = answer.Refresh
	}
	s.ExpiresAt = tokenClaims(answer.Access).expiry()
	if answer.Lifetime.Duration != nil {
		s.ExpiresAt = tokenExpiry(a.clock.Now(), *answer.Lifetime.Duration)
	}
	return s, nil
}

// tokenExpiry converts a token lifetime into the stored timestamp.
func tokenExpiry(now core.Timestamp, lifetime time.Duration) *core.Timestamp {
	at, err := now.Time()
	if err != nil {
		return nil
	}
	expiry, err := core.TimestampFromTime(at.Add(lifetime))
	if err != nil {
		return nil
	}
	return &expiry
}

// withoutURL keeps issuer and proxy addresses out of local error messages.
func withoutURL(err error) error {
	if e, ok := err.(*url.Error); ok {
		return e.Err
	}
	return err
}
