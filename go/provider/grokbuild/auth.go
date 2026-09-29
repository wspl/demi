package grokbuild

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/provider"
)

//demi:wire
type SecretDocument struct {
	AccessToken  provider.Secret  `json:"accessToken" check:"func=provider.Validate"`
	RefreshToken *provider.Secret `json:"refreshToken,omitzero" check:"func=provider.Validate"`
	ExpiresAt    *core.Timestamp  `json:"expiresAt,omitzero" check:"func=core.Validate"`
	Issuer       string           `json:"issuer" check:"func=issuer"`
	ClientID     string           `json:"clientId"`
	Principal    *Principal       `json:"principal,omitzero"`
	UserID       *string          `json:"userId,omitzero"`
	Email        *string          `json:"email,omitzero"`
}

//demi:wire
type Principal struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}

func issuer(text string) error {
	u, err := url.Parse(text)
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") {
		return &wire.InvalidError{Rule: "an issuer is an http or https URL"}
	}
	return nil
}
func (s SecretDocument) JSON() ([]byte, error) { return encode(s) }
func decodeSecret(raw []byte) (SecretDocument, error) {
	return provider.DecodeSecret(raw, decode[SecretDocument])
}

//demi:wire open
type claims struct {
	Exp                *provider.ReportedNumber `json:"exp,omitzero" check:"nullabsent,func=provider.Validate"`
	Sub                *provider.ReportedString `json:"sub,omitzero" check:"nullabsent,func=provider.Validate"`
	Email              *provider.ReportedString `json:"email,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalType      *provider.ReportedString `json:"principal_type,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalTypeCamel *provider.ReportedString `json:"principalType,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalID        *provider.ReportedString `json:"principal_id,omitzero" check:"nullabsent,func=provider.Validate"`
	PrincipalIDCamel   *provider.ReportedString `json:"principalId,omitzero" check:"nullabsent,func=provider.Validate"`
}

func claimsOf(token provider.Secret) claims {
	v, _ := provider.JWTClaims(token.Expose(), decode[claims])
	return v
}
func reported(v *provider.ReportedString) *string {
	if v == nil {
		return nil
	}
	return v.Value
}
func (c claims) principal() *Principal {
	kind, id := reported(c.PrincipalType), reported(c.PrincipalID)
	if kind == nil {
		kind = reported(c.PrincipalTypeCamel)
	}
	if id == nil {
		id = reported(c.PrincipalIDCamel)
	}
	if kind == nil || id == nil {
		return nil
	}
	return &Principal{Kind: *kind, ID: *id}
}
func (c claims) expiry() *core.Timestamp {
	if c.Exp == nil || c.Exp.Value == nil {
		return nil
	}
	return provider.UnixSeconds(*c.Exp.Value)
}
func (s SecretDocument) label() provider.AccountLabel {
	identity := strings.TrimRight(s.Issuer, "/") + "::"
	display := s.ClientID
	if s.UserID != nil {
		display = *s.UserID
	} else if s.Email != nil {
		display = *s.Email
	}
	identity += display
	display = identity
	if s.Email != nil {
		display = *s.Email
	} else if s.UserID != nil {
		display = *s.UserID
	}
	return provider.AccountLabel{Label: display, Detail: new("oidc"), IdentityKey: &identity}
}
func (p *Provider) document() (provider.AccountDocument, *provider.AuthFailure) {
	if p.config.Account == nil {
		return nil, &provider.AuthFailure{Family: "Grok", Reason: provider.AuthReasonMissing}
	}
	return p.pool.Document(*p.config.Account), nil
}
func (p *Provider) stored(ctx context.Context) (SecretDocument, *provider.AuthFailure) {
	doc, failure := p.document()
	if failure != nil {
		return SecretDocument{}, failure
	}
	stored, err := provider.ReadSecret(ctx, doc, decodeSecret)
	if err != nil {
		return SecretDocument{}, provider.AccountFailure("Grok", err)
	}
	return stored.Secret, nil
}
func (p *Provider) credentials(ctx context.Context, http *http.Client, refused *provider.Secret) (SecretDocument, *provider.AuthFailure) {
	doc, failure := p.document()
	if failure != nil {
		return SecretDocument{}, failure
	}
	secret, err := provider.Renew(ctx, doc, decodeSecret, encode[SecretDocument], func(s SecretDocument) bool {
		expiry := s.ExpiresAt
		if expiry == nil {
			expiry = claimsOf(s.AccessToken).expiry()
		}
		return s.RefreshToken != nil && (refused != nil && s.AccessToken == *refused || expiry != nil && expiry.Millisecond()-p.clock.Now().Millisecond() <= 300000)
	}, func(ctx context.Context, s SecretDocument) (SecretDocument, error) {
		next, err := p.refresh(ctx, http, s)
		if err != nil {
			return SecretDocument{}, &provider.AuthFailure{Family: "Grok", Reason: provider.AuthReasonRefresh, Detail: err.Error()}
		}
		return next, nil
	})
	if err != nil {
		return SecretDocument{}, provider.AccountFailure("Grok", err)
	}
	return secret, nil
}

//demi:wire open
type tokens struct {
	AccessToken  provider.Secret    `json:"access_token" check:"func=provider.Validate"`
	RefreshToken *provider.Secret   `json:"refresh_token,omitzero" check:"nullabsent,func=provider.Validate"`
	ExpiresIn    *provider.Lifetime `json:"expires_in,omitzero" check:"nullabsent,func=provider.Validate"`
	IDToken      *provider.Secret   `json:"id_token,omitzero" check:"nullabsent,func=provider.Validate"`
}

func tokenExpiry(tokens tokens, now core.Timestamp) *core.Timestamp {
	if tokens.ExpiresIn == nil || tokens.ExpiresIn.Seconds == nil {
		return nil
	}
	return provider.UnixSeconds(float64(now.Millisecond())/1000 + *tokens.ExpiresIn.Seconds)
}
func (p *Provider) refresh(ctx context.Context, client *http.Client, s SecretDocument) (SecretDocument, error) {
	issuer, err := url.Parse(s.Issuer)
	if err != nil {
		return SecretDocument{}, fmt.Errorf("Grok issuer is invalid")
	}
	endpoint, err := provider.EndpointURL(issuer, "/oauth2/token")
	if err != nil {
		return SecretDocument{}, err
	}
	form := url.Values{"grant_type": {"refresh_token"}, "refresh_token": {s.RefreshToken.Expose()}, "client_id": {s.ClientID}}
	if s.Principal != nil {
		form.Set("principal_type", s.Principal.Kind)
		form.Set("principal_id", s.Principal.ID)
	}
	response, err := send(ctx, client, http.MethodPost, endpoint.String(), http.Header{"Accept": {"application/json"}, "Content-Type": {"application/x-www-form-urlencoded"}}, []byte(form.Encode()))
	if err != nil {
		return SecretDocument{}, provider.TransportFailure("Grok token refresh", err)
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return SecretDocument{}, fmt.Errorf("Grok token refresh failed with HTTP %d", response.StatusCode)
	}
	token, err := provider.DecodeOAuthResponse(response, decode[tokens])
	if err != nil {
		return SecretDocument{}, err
	}
	s.AccessToken = token.AccessToken
	if token.RefreshToken != nil {
		s.RefreshToken = token.RefreshToken
	}
	s.ExpiresAt = tokenExpiry(token, p.clock.Now())
	if token.ExpiresIn == nil || token.ExpiresIn.Seconds == nil {
		s.ExpiresAt = claimsOf(token.AccessToken).expiry()
	}
	return s, nil
}
