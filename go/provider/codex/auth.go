package codex

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"fmt"
	"net/http"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/provider"
)

const ClientID = "app_EMoamEEZ73f0CkXaXp7hrann"

//demi:wire
type SecretDocument struct {
	AccessToken  provider.Secret `json:"accessToken" check:"func=provider.Validate"`
	RefreshToken provider.Secret `json:"refreshToken" check:"func=provider.Validate"`
	IDToken      provider.Secret `json:"idToken" check:"func=provider.Validate"`
	AccountID    string          `json:"accountId" check:"func=accountID"`
	LastRefresh  core.Timestamp  `json:"lastRefresh" check:"func=core.Validate"`
}

func accountID(id string) error {
	if id == "" || !provider.HeaderText(id) {
		return &wire.InvalidError{Rule: "an account id is nonempty header text"}
	}
	return nil
}

//demi:wire open
type claims struct {
	Exp     *provider.ReportedNumber `json:"exp,omitzero" check:"nullabsent,func=provider.Validate"`
	Email   *provider.ReportedString `json:"email,omitzero" check:"nullabsent,func=provider.Validate"`
	Auth    *reportedAuth            `json:"https://api.openai.com/auth,omitzero" check:"nullabsent"`
	Profile *reportedProfile         `json:"https://api.openai.com/profile,omitzero" check:"nullabsent"`
}

//demi:wire open
type authClaims struct {
	AccountID *provider.ReportedString `json:"chatgpt_account_id,omitzero" check:"nullabsent,func=provider.Validate"`
	Fedramp   *provider.ReportedBool   `json:"chatgpt_account_is_fedramp,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:wire open
type profileClaims struct {
	Email *provider.ReportedString `json:"email,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:opaque
type reportedAuth struct{ value *authClaims }

func (v *reportedAuth) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.value = nil
	if value, err := decode[authClaims](raw); err == nil {
		v.value = &value
	}
	return nil
}

//demi:opaque
type reportedProfile struct{ value *profileClaims }

func (v *reportedProfile) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.value = nil
	if value, err := decode[profileClaims](raw); err == nil {
		v.value = &value
	}
	return nil
}

type tokenAccount struct {
	accountID, email *string
	fedramp          bool
}

func tokenClaims(token provider.Secret) claims {
	value, _ := provider.JWTClaims(token.Expose(), decode[claims])
	return value
}
func accountOf(token provider.Secret) tokenAccount {
	claims := tokenClaims(token)
	result := tokenAccount{email: reported(claims.Email)}
	if claims.Auth != nil && claims.Auth.value != nil {
		result.accountID = reported(claims.Auth.value.AccountID)
		if value := claims.Auth.value.Fedramp; value != nil && value.Value != nil {
			result.fedramp = *value.Value
		}
	}
	if result.email == nil && claims.Profile != nil && claims.Profile.value != nil {
		result.email = reported(claims.Profile.value.Email)
	}
	return result
}
func reported(value *provider.ReportedString) *string {
	if value == nil {
		return nil
	}
	return value.Value
}
func (s SecretDocument) label() provider.AccountLabel {
	label := s.AccountID
	if email := accountOf(s.IDToken).email; email != nil {
		label = *email
	} else if email := accountOf(s.AccessToken).email; email != nil {
		label = *email
	}
	return provider.AccountLabel{Label: label, Detail: new("chatgpt"), IdentityKey: &s.AccountID}
}
func (s SecretDocument) due(now core.Timestamp) bool {
	if exp := tokenClaims(s.AccessToken).Exp; exp != nil && exp.Value != nil {
		if expiry := provider.UnixSeconds(*exp.Value); expiry != nil && expiry.Millisecond()-now.Millisecond() <= 300000 {
			return true
		}
	}
	return now.Millisecond()-s.LastRefresh.Millisecond() >= 691200000
}
func decodeSecret(raw []byte) (SecretDocument, error) {
	return provider.DecodeSecret(raw, decode[SecretDocument])
}

type auth struct {
	pool     provider.CredentialPool
	account  *string
	tokenURL string
	clock    core.Clock
}

func (a *auth) document() (provider.AccountDocument, *provider.AuthFailure) {
	if a.account == nil {
		return nil, &provider.AuthFailure{Family: "Codex", Reason: provider.AuthReasonMissing}
	}
	return a.pool.Document(*a.account), nil
}
func (a *auth) stored(ctx context.Context) (SecretDocument, *provider.AuthFailure) {
	document, failure := a.document()
	if failure != nil {
		return SecretDocument{}, failure
	}
	stored, err := provider.ReadSecret(ctx, document, decodeSecret)
	if err != nil {
		return SecretDocument{}, provider.AccountFailure("Codex", err)
	}
	return stored.Secret, nil
}
func (a *auth) credentials(ctx context.Context, client *http.Client, refused *provider.Secret) (SecretDocument, *provider.AuthFailure) {
	document, failure := a.document()
	if failure != nil {
		return SecretDocument{}, failure
	}
	secret, err := provider.Renew(ctx, document, decodeSecret, encode[SecretDocument], func(secret SecretDocument) bool {
		return secret.due(a.clock.Now()) || refused != nil && secret.AccessToken == *refused
	}, func(ctx context.Context, secret SecretDocument) (SecretDocument, error) {
		next, err := a.refresh(ctx, client, secret)
		if err != nil {
			return SecretDocument{}, &provider.AuthFailure{Family: "Codex", Reason: provider.AuthReasonRefresh, Detail: err.Error()}
		}
		return next, nil
	})
	if err != nil {
		return SecretDocument{}, provider.AccountFailure("Codex", err)
	}
	return secret, nil
}

//demi:wire
type refreshRequest struct {
	ClientID     string          `json:"client_id"`
	GrantType    string          `json:"grant_type"`
	RefreshToken provider.Secret `json:"refresh_token" check:"func=provider.Validate"`
}

//demi:wire open
type refreshedTokens struct {
	AccessToken  provider.Secret  `json:"access_token" check:"func=provider.Validate"`
	RefreshToken *provider.Secret `json:"refresh_token,omitzero" check:"nullabsent,func=provider.Validate"`
	IDToken      *provider.Secret `json:"id_token,omitzero" check:"nullabsent,func=provider.Validate"`
}

func (a *auth) refresh(ctx context.Context, client *http.Client, secret SecretDocument) (SecretDocument, error) {
	body := provider.JSONBody(refreshRequest{ClientID: ClientID, GrantType: "refresh_token", RefreshToken: secret.RefreshToken})
	response, err := post(ctx, client, a.tokenURL, "application/json", body)
	if err != nil {
		return SecretDocument{}, fmt.Errorf("Codex token refresh failed: %w", err)
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return SecretDocument{}, fmt.Errorf("Codex token refresh failed with HTTP %d", response.StatusCode)
	}
	tokens, err := provider.DecodeOAuthResponse(response, decode[refreshedTokens])
	if err != nil {
		return SecretDocument{}, fmt.Errorf("Codex token refresh failed: %w", err)
	}
	secret.AccessToken = tokens.AccessToken
	if tokens.RefreshToken != nil {
		secret.RefreshToken = *tokens.RefreshToken
	}
	if tokens.IDToken != nil {
		secret.IDToken = *tokens.IDToken
	}
	secret.LastRefresh = a.clock.Now()
	return secret, nil
}
func post(ctx context.Context, client *http.Client, endpoint, contentType string, body []byte) (*http.Response, error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(body))
	if err != nil {
		return nil, fmt.Errorf("Codex sign-in request could not be built")
	}
	request.Header.Set("Content-Type", contentType)
	response, err := client.Do(request)
	if err != nil {
		return nil, provider.TransportFailure("Codex sign-in", err)
	}
	return response, nil
}

// Keep JSON's custom secret methods on the exported document at external boundaries.
func (s SecretDocument) JSON() ([]byte, error) { return encode(s) }
